//! `manivelle run`: starts the target of a package, with its packages or without, and stays
//! connected to it.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use clap::Args;
use manivelle::PackageManifest;
use manivelle::protocol::{
    Command, FromProgram, PACKAGES_ENV, PROTOCOL_ENV, Reply, Request, TOKEN_ENV,
};

use super::Failure;
use super::roots::canonical_roots;

/// How often `--watch` asks the target to reload the files that changed.
const WATCH_EVERY: Duration = Duration::from_secs(1);

#[derive(Args)]
pub struct RunArgs {
    /// The package's directory, with its `manivelle.kdl`
    #[arg(default_value = ".")]
    directory: PathBuf,
    /// The program to start, instead of the manifest's `target`
    #[arg(long)]
    target: Option<PathBuf>,
    /// Start the target without any package
    #[arg(long, conflicts_with = "package_dirs")]
    no_packages: bool,
    /// Also load the packages in DIR (a package, or a directory of packages); repeat for several
    #[arg(long = "packages", value_name = "DIR")]
    package_dirs: Vec<PathBuf>,
    /// Reload the packages' files while the target runs, when they change
    #[arg(long)]
    watch: bool,
    /// Seconds to wait for the target to connect before leaving it to run on its own
    #[arg(long, default_value_t = 30)]
    timeout: u64,
    /// Arguments for the target, after `--`
    #[arg(last = true)]
    program_args: Vec<String>,
}

/// What to start and which packages to give it.
struct Plan {
    target: PathBuf,
    package_roots: Vec<PathBuf>,
}

pub fn run_target(args: RunArgs) -> Result<(), Failure> {
    let plan = plan(&args)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let token = new_token()?;
    let mut child = spawn_target(&plan, &args.program_args, listener.local_addr()?, &token)?;

    let status = match await_target(&listener, &mut child, &token, args.timeout) {
        Ok(Some(connection)) => attended(connection, &mut child, args.watch)?,
        Ok(None) => {
            eprintln!(
                "[manivelle] {} did not connect within {} s: does it call \
                 `Engine::setup_protocol`? It keeps running without packages.",
                plan.target.display(),
                args.timeout
            );
            child.wait()?
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    exit_outcome(&plan.target, status)
}

/// Starts the target with the address to connect to, the token to show and the packages.
fn spawn_target(
    plan: &Plan,
    program_args: &[String],
    address: SocketAddr,
    token: &str,
) -> Result<Child, Failure> {
    let mut command = Process::new(&plan.target);
    command
        .args(program_args)
        .env(PROTOCOL_ENV, address.to_string())
        .env(TOKEN_ENV, token);
    if plan.package_roots.is_empty() {
        command.env_remove(PACKAGES_ENV);
    } else {
        command.env(PACKAGES_ENV, std::env::join_paths(&plan.package_roots)?);
    }
    println!(
        "[manivelle] running {} with {} package location(s)",
        plan.target.display(),
        plan.package_roots.len()
    );
    let child = command
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", plan.target.display()))?;
    Ok(child)
}

/// Succeeds when the target exited successfully.
fn exit_outcome(target: &Path, status: ExitStatus) -> Result<(), Failure> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", target.display()).into())
    }
}

fn plan(args: &RunArgs) -> Result<Plan, Failure> {
    let manifest = match PackageManifest::read(&args.directory) {
        Ok(manifest) => Some(manifest),
        Err(_) if args.no_packages && args.target.is_some() => None,
        Err(error) => return Err(error.into()),
    };
    let target = match (&args.target, &manifest) {
        (Some(target), _) => target.clone(),
        (None, Some(manifest)) => match &manifest.target {
            Some(target) => manifest.root.join(target),
            None => return Err(no_target()),
        },
        (None, None) => return Err(no_target()),
    };
    let target = fs::canonicalize(&target)
        .map_err(|error| format!("cannot find the target {}: {error}", target.display()))?;

    let package_roots = if args.no_packages {
        Vec::new()
    } else {
        let own = manifest.iter().map(|manifest| manifest.root.clone());
        canonical_roots(own.chain(args.package_dirs.iter().cloned()))?
    };
    Ok(Plan {
        target,
        package_roots,
    })
}

fn no_target() -> Failure {
    "no target to run: add `target \"../program\"` to the manifest, or pass --target".into()
}

/// A secret the target must show to be heard: 256 bits from the operating system's random
/// source, in hex.
fn new_token() -> Result<String, Failure> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("cannot read the operating system's randomness: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The connection of the target, once it showed the token.
struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

/// Waits for the target to connect and show `token`, for `timeout` seconds. Anything
/// else that connects to the port is dropped. `None` when the time passed; an error when
/// the target exited first.
fn await_target(
    listener: &TcpListener,
    child: &mut Child,
    token: &str,
    timeout: u64,
) -> Result<Option<Connection>, Failure> {
    listener.set_nonblocking(true)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(timeout) {
        // Read before accepting: a target that connected and exited has its connection
        // queued by then.
        let exited = child.try_wait()?;
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(connection) = greeted(stream, token) {
                    return Ok(Some(connection));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if let Some(status) = exited {
                    return Err(format!(
                        "the target exited with {status} before connecting: \
                         it must call `Engine::setup_protocol` at startup"
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}

/// The connection of `stream` when it opens with the right `hello`.
fn greeted(stream: TcpStream, token: &str) -> Option<Connection> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    match serde_json::from_str(&line) {
        Ok(FromProgram::Hello { token: shown, .. }) if shown == token => {
            stream.set_read_timeout(None).ok()?;
            Some(Connection {
                reader,
                writer: stream,
            })
        }
        _ => None,
    }
}

/// Reports what the target says until it exits, asking it to reload when `watch`.
fn attended(connection: Connection, child: &mut Child, watch: bool) -> Result<ExitStatus, Failure> {
    let Connection { reader, mut writer } = connection;
    let printer = thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(message) = serde_json::from_str::<FromProgram>(&line) {
                report(&message);
            }
        }
    });

    let mut next_id = 0;
    let mut asked = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if watch && asked.elapsed() >= WATCH_EVERY {
            asked = Instant::now();
            next_id += 1;
            let command = Command {
                id: next_id,
                request: Request::Reload,
            };
            // A target that stopped listening is not an error: it keeps running.
            let _ = writeln!(writer, "{}", serde_json::to_string(&command)?);
        }
        thread::sleep(Duration::from_millis(20));
    };
    let _ = printer.join();
    Ok(status)
}

fn report(message: &FromProgram) {
    match message {
        FromProgram::Hello { .. } => {}
        FromProgram::Console {
            level,
            script,
            message,
        } => {
            // Warnings and errors go to stderr, as the target would have written them.
            if matches!(level.as_str(), "warn" | "error") {
                eprintln!("[{level}] {script}: {message}");
            } else {
                println!("[{level}] {script}: {message}");
            }
        }
        FromProgram::Packages { loaded, failed } => {
            for module in loaded {
                println!(
                    "[manivelle] package {} {} loaded",
                    module.name, module.version
                );
            }
            for module in failed {
                eprintln!(
                    "[manivelle] package {} failed: {}",
                    module.name, module.reason
                );
            }
        }
        FromProgram::Fatal { message } => {
            eprintln!("[manivelle] cannot load the packages: {message}")
        }
        FromProgram::Reply { reply, .. } => match reply {
            Reply::Reloaded { reloaded, failed } => {
                for name in reloaded {
                    println!("[manivelle] reloaded {name}");
                }
                for module in failed {
                    eprintln!(
                        "[manivelle] reload of {} failed: {}",
                        module.name, module.reason
                    );
                }
            }
            Reply::Error { message } => eprintln!("[manivelle] the target refused: {message}"),
            Reply::Done | Reply::Contracts { .. } | Reply::Packages { .. } => {}
        },
    }
}
