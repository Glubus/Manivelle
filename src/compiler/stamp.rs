//! File change detection for hot reload, without watcher threads.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Size and modification time of one file or directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    /// `None` when the path does not exist (anymore).
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Paths a loaded project depends on, stamped when the load read them.
///
/// Module files are stamped before they are read, so an edit racing the load is seen
/// as a change on the next check. Directories are watched too: adding a file next to a
/// module can change what an import resolves to.
#[derive(Debug, Clone, Default)]
pub(crate) struct WatchedFiles(BTreeMap<PathBuf, Option<FileStamp>>);

impl WatchedFiles {
    /// Stamps `path` unless it is already watched.
    pub(crate) fn watch(&mut self, path: &Path) {
        if !self.0.contains_key(path) {
            self.0.insert(path.to_path_buf(), FileStamp::of(path));
        }
    }

    /// Watches `path` with a stamp taken earlier, before the file was read.
    pub(crate) fn watch_stamped(&mut self, path: &Path, stamp: Option<FileStamp>) {
        self.0.insert(path.to_path_buf(), stamp);
    }

    /// Whether any watched path was modified, created or removed since it was stamped.
    pub(crate) fn changed(&self) -> bool {
        self.0
            .iter()
            .any(|(path, stamp)| FileStamp::of(path) != *stamp)
    }

    /// Takes the current state as the new reference.
    pub(crate) fn restamp(&mut self) {
        for (path, stamp) in &mut self.0 {
            *stamp = FileStamp::of(path);
        }
    }

    /// Watches every path of `other` with its stamp. A path both watch takes the stamp of
    /// `other`.
    pub(crate) fn merge(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}
