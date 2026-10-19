//! The link between a program that embeds Manivelle and the `manivelle` tool.
//!
//! A program calls [`Engine::setup_protocol`] once at startup and
//! [`Engine::poll_protocol`] once per frame, next to `pump`. When the tool started the
//! program (`manivelle init`, `manivelle run`) they do what the tool asked: export the
//! SDK, load the packages, answer requests. When nothing started it, they do nothing.

mod link;
mod wire;

use std::path::{Path, PathBuf};

pub(crate) use link::Link;
pub use wire::{
    Command, FromProgram, PACKAGES_ENV, PROTOCOL_ENV, PackageSummary, Reply, Request, TOKEN_ENV,
};

use crate::contract::HostContractKind;
use crate::error::EngineError;
use crate::packages::{PackageFailure, PackageLoad};
use crate::runner::Engine;
use crate::sdk_files::{export_sdk, export_sdk_if_requested};

/// What [`Engine::setup_protocol`] found and did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Setup {
    /// The tool asked for the SDK (`manivelle init`) and it is written: the program
    /// should exit without starting.
    pub exported: bool,
    /// The program is connected to the tool and answers its requests in
    /// [`Engine::poll_protocol`].
    pub attached: bool,
    /// The packages the tool asked to load; empty when it asked for none (`--no-packages`) or
    /// nothing started the program.
    pub packages: PackageLoad,
}

impl Engine {
    /// Does what the `manivelle` tool that started the program asked at startup. Call
    /// it once, after registering the host contracts and before anything else the
    /// program starts (a window, assets):
    ///
    /// ```no_run
    /// # fn main() -> Result<(), manivelle::EngineError> {
    /// let mut engine = manivelle::Engine::new(&manivelle::EngineOptions::default())?;
    /// // engine.registry().function::<MyContract>()?; ...
    /// if engine.setup_protocol()?.exported {
    ///     return Ok(()); // `manivelle init`: the SDK is written, nothing else to do
    /// }
    /// loop {
    ///     engine.poll_protocol(); // answers the tool's requests, almost free when idle
    ///     engine.pump()?;
    ///     // ... the frame
    /// #   break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// In order:
    ///
    /// 1. A request for the SDK (what `export_sdk_if_requested` reads) writes it and
    ///    sets [`Setup::exported`].
    /// 2. When [`PROTOCOL_ENV`] names the tool, connects to it, showing the secret of
    ///    [`TOKEN_ENV`]. The connection starts one thread, which only reads requests
    ///    and queues them; they run on the engine's thread in
    ///    [`poll_protocol`](Self::poll_protocol).
    /// 3. When [`PACKAGES_ENV`] lists directories, loads their packages with
    ///    [`load_packages`](Self::load_packages) and tells the tool what loaded.
    ///
    /// Started by hand, none of those variables is set and nothing happens: a shipped
    /// program that wants packages calls `load_packages` itself.
    ///
    /// # Errors
    ///
    /// Fails when the SDK cannot be written, the tool cannot be reached, or the packages
    /// cannot be loaded together (the tool is told first).
    pub fn setup_protocol(&mut self) -> Result<Setup, EngineError> {
        if export_sdk_if_requested(self.registry())? {
            return Ok(Setup {
                exported: true,
                ..Setup::default()
            });
        }
        self.protocol = connect_from_env()?;
        if let Some(outbox) = self.protocol.as_ref().map(Link::outbox) {
            self.forward_console(move |level, script, message| {
                // A full outbox drops the line; the target never waits for the tool.
                let _ = outbox.try_send(FromProgram::Console {
                    level: level.to_string(),
                    script: script.to_owned(),
                    message: message.to_owned(),
                });
            });
        }
        let mut setup = Setup {
            attached: self.protocol.is_some(),
            ..Setup::default()
        };

        let roots = package_roots();
        if roots.is_empty() {
            return Ok(setup);
        }
        match self.load_packages(&roots) {
            Ok(load) => {
                self.tell(FromProgram::Packages {
                    loaded: load.loaded.iter().map(PackageSummary::from).collect(),
                    failed: load.failed.clone(),
                });
                setup.packages = load;
                Ok(setup)
            }
            Err(error) => {
                self.tell(FromProgram::Fatal {
                    message: error.to_string(),
                });
                Err(error)
            }
        }
    }

    /// Runs the requests the `manivelle` tool queued since the last call, and answers
    /// them; returns how many. Call it once per frame, from the thread that owns the
    /// engine, next to `pump`: with no request waiting, and with no tool attached, it
    /// is one atomic read. A request that fails is answered with the error; it never
    /// fails the program.
    pub fn poll_protocol(&mut self) -> usize {
        let commands = match &self.protocol {
            Some(link) if link.is_alive() => link.take_commands(),
            Some(_) => {
                self.drop_protocol();
                return 0;
            }
            None => return 0,
        };
        let handled = commands.len();
        for command in commands {
            let reply = self.run_request(command.request);
            self.tell(FromProgram::Reply {
                id: command.id,
                reply,
            });
        }
        handled
    }

    fn run_request(&mut self, request: Request) -> Reply {
        match request {
            Request::ExportSdk { dir } => match export_sdk(self.registry(), Path::new(&dir)) {
                Ok(()) => Reply::Done,
                Err(error) => failed(&error),
            },
            Request::ListContracts => match self.registry().descriptors() {
                Ok(descriptors) => Reply::Contracts {
                    names: descriptors
                        .into_iter()
                        .filter(|d| d.kind != HostContractKind::Export)
                        .map(|d| d.name)
                        .collect(),
                },
                Err(error) => failed(&error),
            },
            Request::ListPackages => Reply::Packages {
                packages: self.packages().iter().map(PackageSummary::from).collect(),
            },
            Request::Reload => {
                let report = self.reload_changed();
                let reason = |label: &str, error: &EngineError| format!("{label}{error}");
                let mut failures: Vec<PackageFailure> = report
                    .failed
                    .iter()
                    .map(|(name, error)| PackageFailure {
                        name: name.to_string(),
                        reason: reason("", error),
                    })
                    .collect();
                failures.extend(
                    report
                        .dispose_failed
                        .iter()
                        .map(|(name, error)| PackageFailure {
                            name: name.to_string(),
                            reason: reason("dispose: ", error),
                        }),
                );
                Reply::Reloaded {
                    reloaded: report.reloaded.iter().map(ToString::to_string).collect(),
                    failed: failures,
                }
            }
        }
    }

    /// Sends `message` to the tool; a tool that cannot be reached any more is let go
    /// rather than stalling or failing the program.
    fn tell(&mut self, message: FromProgram) {
        if let Some(link) = &self.protocol
            && link.send(message).is_err()
        {
            self.drop_protocol();
        }
    }

    /// Lets go of a tool that is gone: its `console` output returns to the program.
    fn drop_protocol(&mut self) {
        self.protocol = None;
        self.stop_forwarding_console();
    }
}

fn failed(error: &EngineError) -> Reply {
    Reply::Error {
        message: error.to_string(),
    }
}

/// The connection [`PROTOCOL_ENV`] and [`TOKEN_ENV`] ask for, if they are set.
fn connect_from_env() -> Result<Option<Link>, EngineError> {
    let Some(address) = std::env::var(PROTOCOL_ENV).ok().filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let token = std::env::var(TOKEN_ENV).unwrap_or_default();
    Ok(Some(Link::connect(&address, &token)?))
}

/// The directories [`PACKAGES_ENV`] lists.
fn package_roots() -> Vec<PathBuf> {
    std::env::var_os(PACKAGES_ENV)
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}
