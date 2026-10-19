//! The messages the `manivelle` tool and a program exchange, one JSON object per line.

use serde::{Deserialize, Serialize};

use crate::packages::{PackageFailure, PackageInfo};

/// The environment variable through which the tool gives a program the `host:port` it
/// listens on.
pub const PROTOCOL_ENV: &str = "MANIVELLE_PROTOCOL";

/// The environment variable holding the secret the program shows in its
/// [`FromProgram::Hello`], so that only the program the tool started is heard.
pub const TOKEN_ENV: &str = "MANIVELLE_TOKEN";

/// The environment variable listing the package directories to load, separated like `PATH`.
pub const PACKAGES_ENV: &str = "MANIVELLE_PACKAGES";

/// A package, as the tool lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSummary {
    /// The package's name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Who wrote it.
    pub author: Option<String>,
    /// What it declares it uses.
    pub permissions: Vec<String>,
}

impl From<&PackageInfo> for PackageSummary {
    fn from(info: &PackageInfo) -> Self {
        Self {
            name: info.name.clone(),
            version: info.version.to_string(),
            author: info.author.clone(),
            permissions: info.permissions.clone(),
        }
    }
}

/// What the program sends the tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromProgram {
    /// The first message: the program shows the secret of its environment.
    Hello {
        /// [`TOKEN_ENV`].
        token: String,
        /// The program's process id.
        pid: u32,
        /// The version of Manivelle the program embeds.
        manivelle: String,
    },
    /// The result of loading the packages [`PACKAGES_ENV`] lists.
    Packages {
        /// Loaded packages, dependencies first.
        loaded: Vec<PackageSummary>,
        /// Packages that did not load.
        failed: Vec<PackageFailure>,
    },
    /// The packages could not be loaded at all; the program stops starting.
    Fatal {
        /// Why.
        message: String,
    },
    /// One `console` call of a script, while the program is attached.
    Console {
        /// `debug`, `log`, `info`, `warn` or `error`.
        level: String,
        /// The id of the script that called it.
        script: String,
        /// The message, formatted as `console` does.
        message: String,
    },
    /// The answer to a [`Command`].
    Reply {
        /// The [`Command::id`] it answers.
        id: u64,
        /// The answer.
        reply: Reply,
    },
}

/// What the tool sends the program: a request to run on the program's own thread, at its
/// next `Engine::poll_protocol`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Echoed by the [`FromProgram::Reply`].
    pub id: u64,
    /// What to do.
    pub request: Request,
}

/// A request to the program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Write the declarations, the SDK module and the descriptors into a directory, as
    /// `export_sdk_if_requested` does.
    ExportSdk {
        /// The directory to write to.
        dir: String,
    },
    /// List the registered host contracts.
    ListContracts,
    /// List the loaded packages.
    ListPackages,
    /// Reload the projects whose files changed, like `Engine::reload_changed`.
    Reload,
}

/// The answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    /// Done, with nothing to report.
    Done,
    /// The names of the registered host contracts.
    Contracts {
        /// The names.
        names: Vec<String>,
    },
    /// The loaded packages.
    Packages {
        /// The packages.
        packages: Vec<PackageSummary>,
    },
    /// What a reload did.
    Reloaded {
        /// The scripts reloaded.
        reloaded: Vec<String>,
        /// The reloads that failed, each script keeping its previous version.
        failed: Vec<PackageFailure>,
    },
    /// The request failed.
    Error {
        /// Why.
        message: String,
    },
}
