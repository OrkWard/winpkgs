//! Paths of the wpk file layout.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Root of all wpk files, overriding `~/wpk`.
pub const ROOT_ENV: &str = "WPK_ROOT";

/// Folder for shortcuts, overriding the Start menu's `wpk` folder.
pub const START_MENU_ENV: &str = "WPK_START_MENU";

/// Name of the package wpk installs itself as; its dir also holds wpk's state.
pub const SELF_PACKAGE: &str = "wpk";

/// Where every wpk file lives.
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
    cache: PathBuf,
    start_menu: PathBuf,
}

impl Layout {
    /// A layout whose shortcuts go to `<root>/start-menu`, for tests and
    /// tools that must not touch the user's Start menu.
    pub fn new(root: impl Into<PathBuf>, cache: impl Into<PathBuf>) -> Layout {
        let root = normalize(root.into());
        Layout {
            start_menu: root.join("start-menu"),
            root,
            cache: normalize(cache.into()),
        }
    }

    /// Root from `WPK_ROOT` or `~/wpk`; cache from `XDG_CACHE_HOME/wpk` or
    /// `~/.cache/wpk`; shortcuts in the Start menu's `wpk` folder.
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
        let mut layout = Layout::new(root, cache);
        if let Some(dir) = non_empty_env(START_MENU_ENV) {
            layout.start_menu = normalize(dir);
        } else if let Some(appdata) = non_empty_env("APPDATA") {
            layout.start_menu = normalize(appdata.join(r"Microsoft\Windows\Start Menu\Programs\wpk"));
        }
        Ok(layout)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// The Start menu folder holding wpk's shortcuts.
    pub fn start_menu(&self) -> &Path {
        &self.start_menu
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

    /// Install dir of one version; characters Windows forbids become `_`.
    pub fn version(&self, name: &str, version: &str) -> PathBuf {
        let safe: String = version
            .chars()
            .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
            .collect();
        self.package(name).join(safe)
    }

    /// wpk's records inside a version dir.
    pub fn records(&self, name: &str, version: &str) -> PathBuf {
        self.version(name, version).join(".wpk")
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

/// An absolute path with `\` separators only: paths built from variables
/// like `TEMP` may mix in `/`, which msiexec and others reject.
fn normalize(path: PathBuf) -> PathBuf {
    let path = std::path::absolute(&path).unwrap_or(path);
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().replace('/', "\\"))
    } else {
        path
    }
}

/// Whether `s` can be a single path component: a package name or version.
pub fn is_safe_name(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.starts_with(".wpk")
        && !s.ends_with(".tmp")
        && !s.contains(['/', '\\'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_lives_in_self_package() {
        let layout = Layout::new("C:/r", "C:/c");
        assert_eq!(layout.config(), Path::new(r"C:\r\opt\wpk\config.toml"));
        assert_eq!(layout.held("fd"), Path::new(r"C:\r\opt\wpk\held\fd"));
        assert_eq!(layout.current("fd"), Path::new(r"C:\r\opt\fd\current"));
        assert_eq!(layout.persist("fd"), Path::new(r"C:\r\var\fd"));
        assert_eq!(layout.version("fd", "1:2"), Path::new(r"C:\r\opt\fd\1_2"));
    }
}
