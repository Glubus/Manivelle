//! Finding the package manifests under the roots given, and checking them together.

use semver::Version;
use std::fs;
use std::path::{Path, PathBuf};

use super::manifest::{MANIFEST_FILE, PackageManifest};
use super::resolve;
use crate::error::EngineError;

/// Reads the packages found in `roots` (each a package, or a directory of packages) and
/// checks the whole set as [`Engine::load_packages`](crate::Engine::load_packages) does, without loading anything:
/// unique names, `engine` requirements, dependencies and their versions, and cycles. The
/// packages come back dependencies first.
///
/// # Errors
///
/// Fails when a root has no package, a manifest is not valid, or the set cannot work together.
pub fn resolve_packages(roots: &[PathBuf]) -> Result<Vec<PackageManifest>, EngineError> {
    let mut manifests = Vec::new();
    for root in roots {
        manifests.extend(discover(root)?);
    }
    let engine_version =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(|error| EngineError::Manifest {
            path: "manivelle".to_owned(),
            details: error.to_string(),
        })?;
    resolve::load_order(manifests, &engine_version)
}

/// The manifests of the package at `root`, or of the packages in its subdirectories.
fn discover(root: &Path) -> Result<Vec<PackageManifest>, EngineError> {
    if root.join(MANIFEST_FILE).is_file() {
        return Ok(vec![PackageManifest::read(root)?]);
    }
    let entries = fs::read_dir(root).map_err(|error| EngineError::Manifest {
        path: root.display().to_string(),
        details: error.to_string(),
    })?;
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(MANIFEST_FILE).is_file())
        .collect();
    directories.sort();
    if directories.is_empty() {
        return Err(EngineError::Manifest {
            path: root.display().to_string(),
            details: format!("no {MANIFEST_FILE} here or in a subdirectory"),
        });
    }
    directories
        .iter()
        .map(|path| PackageManifest::read(path))
        .collect()
}
