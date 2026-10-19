//! Writing the files of a project without following links out of it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use super::Failure;

/// Writes `content` to `path`, replacing what is there. The file is written beside it and
/// renamed over it, so a symbolic link at `path` is replaced rather than followed to
/// whatever it points at.
pub(super) fn replace_file(path: &Path, content: &[u8]) -> Result<(), Failure> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Creates `path` with `content` unless something is there already, a dangling symbolic
/// link included (`create_new` never follows one).
pub(super) fn create_if_missing(path: &Path, content: &str) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes())?;
            println!("created {}", path.display());
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}
