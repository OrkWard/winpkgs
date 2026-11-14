//! `std::fs` with errors that name the path.

use std::path::Path;

use crate::{Error, Result};

pub(crate) fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

pub fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(io_error(path))
}

/// Remove a directory tree; a missing one is fine.
pub fn remove_dir_all(path: &Path) -> Result<()> {
    match remove_dir_all::remove_dir_all(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other.map_err(io_error(path)),
    }
}

/// Remove a file; a missing one is fine.
pub fn remove_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other.map_err(io_error(path)),
    }
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).map_err(io_error(from))
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(io_error(path))
}

pub fn read_to_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(io_error(path))
}

/// Write through a temporary file and rename it over `path`, so readers see
/// the old or the new content, never a mix.
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    std::fs::write(tmp, content).map_err(io_error(tmp))?;
    std::fs::rename(tmp, path).map_err(io_error(path))
}
