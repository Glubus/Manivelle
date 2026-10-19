//! Packages: scripts packaged with a `manivelle.kdl` manifest that says what they need.

mod discover;
mod edit;
mod manifest;
mod names;
mod permissions;
mod resolve;

pub use discover::resolve_packages;
pub use edit::add_permissions;
pub use manifest::{MANIFEST_FILE, PackageDependency, PackageManifest};
pub(crate) use names::{closest, permission_matches};
pub(crate) use permissions::PermissionSet;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::EngineError;
use crate::runner::Engine;

/// A package the engine loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageInfo {
    /// The package's name, which is also the id of its script.
    pub name: String,
    /// Its version.
    pub version: Version,
    /// Who wrote it.
    pub author: Option<String>,
    /// The host contracts it may use: names, `namespace.*` or `*`.
    pub permissions: Vec<String>,
    /// Its directory.
    pub root: PathBuf,
}

/// A package that did not load, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFailure {
    /// The package's name.
    pub name: String,
    /// What went wrong: its own error, or the dependency that did not load.
    pub reason: String,
}

/// What [`Engine::load_packages`](crate::Engine::load_packages) did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageLoad {
    /// The packages now loaded, dependencies first.
    pub loaded: Vec<PackageInfo>,
    /// The packages that failed to load. A package whose dependency failed is listed here too,
    /// without having been tried.
    pub failed: Vec<PackageFailure>,
}

impl Engine {
    /// Loads the packages found in `roots`, dependencies first.
    ///
    /// Each root is either a package (it holds a [`MANIFEST_FILE`]) or a directory of packages
    /// (each of its subdirectories with a manifest is one). Every manifest is read and
    /// the whole set checked (names, `engine` requirements, dependencies and their
    /// versions, cycles) before any package loads, so a set that cannot work together loads
    /// nothing. Each package then loads as its own project, from its `entry`, with its name
    /// as the script id and a context of its own; one that fails to load does not stop
    /// the others, except the packages that depend on it.
    ///
    /// A package's name is its script id: loading a package replaces a script of the same id.
    /// The `permissions` of a manifest are enforced: the package's script can only call
    /// the host functions, and listen to the events, they name (a package that names none
    /// reaches no host contract). A script loaded with `load_script` or `load_project` is
    /// not restricted.
    ///
    /// # Errors
    ///
    /// Fails when a root has no package, a manifest is not valid, or the set of packages cannot
    /// be loaded together; nothing is loaded then.
    pub fn load_packages(&mut self, roots: &[PathBuf]) -> Result<PackageLoad, EngineError> {
        let ordered = resolve_packages(roots)?;

        let mut report = PackageLoad::default();
        for manifest in ordered {
            let blocked = manifest
                .depends
                .iter()
                .find(|dependency| report.failed.iter().any(|f| f.name == dependency.name));
            if let Some(dependency) = blocked {
                report.failed.push(PackageFailure {
                    name: manifest.name,
                    reason: format!("its dependency `{}` did not load", dependency.name),
                });
                continue;
            }
            let permissions = PermissionSet::new(&manifest.permissions);
            match self.load_project_restricted(
                manifest.name.as_str(),
                manifest.entry_path(),
                permissions,
            ) {
                Ok(()) => {
                    let info = PackageInfo {
                        name: manifest.name,
                        version: manifest.version,
                        author: manifest.author,
                        permissions: manifest.permissions,
                        root: manifest.root,
                    };
                    self.packages.retain(|known| known.name != info.name);
                    self.packages.push(info.clone());
                    report.loaded.push(info);
                }
                Err(error) => report.failed.push(PackageFailure {
                    name: manifest.name,
                    reason: error.to_string(),
                }),
            }
        }
        Ok(report)
    }

    /// The packages [`Engine::load_packages`](crate::Engine::load_packages) loaded, in load order.
    #[must_use]
    pub fn packages(&self) -> &[PackageInfo] {
        &self.packages
    }
}
