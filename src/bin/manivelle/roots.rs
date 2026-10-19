//! The package directories the commands work on.

use std::fs;
use std::path::PathBuf;

use super::Failure;

/// The canonical form of each of `paths`, in order, with the repeats dropped.
pub(super) fn canonical_roots(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, Failure> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in paths {
        let root = fs::canonicalize(&path)
            .map_err(|error| format!("cannot find {}: {error}", path.display()))?;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
}
