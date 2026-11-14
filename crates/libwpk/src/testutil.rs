//! Helpers for unit tests.

use std::path::{Path, PathBuf};

/// A fresh directory under the system temp dir, deleted on drop. Links
/// inside are removed, not followed.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("wpk-{tag}-{}", std::process::id()));
        let _ = remove_dir_all::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = remove_dir_all::remove_dir_all(&self.0);
    }
}
