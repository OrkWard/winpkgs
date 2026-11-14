//! Paths of the wpk file layout.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Root of all wpk files, overriding `~/wpk`.
pub const ROOT_ENV: &str = "WPK_ROOT";

/// Name of the package wpk installs itself as; its dir also holds wpk's state.
pub const SELF_PACKAGE: &str = "wpk";

/// Where every wpk file lives.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
    cache: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>, cache: impl Into<PathBuf>) -> Layout {
        Layout {
            root: root.into(),
            cache: cache.into(),
        }
    }

    /// Root from `WPK_ROOT` or `~/wpk`; cache from `XDG_CACHE_HOME/wpk` or
    /// `~/.cache/wpk`.
    pub fn from_env() -> Result<Layout> {
        let home = || std::env::home_dir().ok_or(Error::NoHome);
        let root = match non_empty_env(ROOT_ENV) {
            Some(root) => root,
            None => home()?.join("wpk"),
        };
        let cache = match non_empty_env("XDG_CACHE_HOME") {
            Some(cache) => cache.join("wpk"),
            None => home()?.join(".cache").join("wpk"),
        };
        Ok(Layout::new(root, cache))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// The directory users add to PATH.
    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn opt(&self) -> PathBuf {
        self.root.join("opt")
    }

    pub fn var(&self) -> PathBuf {
        self.root.join("var")
    }

    pub fn package(&self, name: &str) -> PathBuf {
        self.opt().join(name)
    }

    pub fn version(&self, name: &str, version: &str) -> PathBuf {
        self.package(name).join(version)
    }

    /// Text file holding the active version of a package.
    pub fn current(&self, name: &str) -> PathBuf {
        self.package(name).join("current")
    }

    /// Persisted data of a package.
    pub fn persist(&self, name: &str) -> PathBuf {
        self.var().join(name)
    }

    /// wpk's own state, next to its installed versions.
    pub fn state(&self) -> PathBuf {
        self.package(SELF_PACKAGE)
    }

    pub fn config(&self) -> PathBuf {
        self.state().join("config.toml")
    }

    pub fn buckets(&self) -> PathBuf {
        self.state().join("buckets")
    }

    pub fn held(&self, name: &str) -> PathBuf {
        self.state().join("held").join(name)
    }

    /// Shims that shortcuts point at, kept out of PATH.
    pub fn launchers(&self, name: &str) -> PathBuf {
        self.state().join("launchers").join(name)
    }

    pub fn lock(&self) -> PathBuf {
        self.state().join("lock")
    }

    pub fn gc_stamp(&self) -> PathBuf {
        self.state().join("gc-stamp")
    }
}

fn non_empty_env(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_lives_in_self_package() {
        let layout = Layout::new("r", "c");
        assert_eq!(layout.config(), Path::new("r/opt/wpk/config.toml"));
        assert_eq!(layout.held("fd"), Path::new("r/opt/wpk/held/fd"));
        assert_eq!(layout.current("fd"), Path::new("r/opt/fd/current"));
        assert_eq!(layout.persist("fd"), Path::new("r/var/fd"));
    }
}
