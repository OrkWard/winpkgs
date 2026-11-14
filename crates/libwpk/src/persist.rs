//! `persist`: data kept in `var/<pkg>` and linked into each version.

use std::path::Path;

use crate::manifest::Persist;
use crate::{Error, Result};

/// Link each persisted path of the version dir `dir` to `var_dir`. Data
/// already in `var_dir` wins over the package's copy, which is kept as
/// `<name>.original`. Data only in the package moves to `var_dir`. When
/// neither has it, an empty file is created if the name has an extension,
/// else an empty directory.
pub fn link(dir: &Path, var_dir: &Path, entries: &[Persist]) -> Result<()> {
    for entry in entries {
        let source = dir.join(&entry.source);
        let target = var_dir.join(&entry.target);
        if let Some(parent) = target.parent() {
            crate::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = source.parent() {
            crate::fs::create_dir_all(parent)?;
        }

        if target.exists() {
            if source.symlink_metadata().is_ok() {
                let mut original = source.as_os_str().to_owned();
                original.push(".original");
                crate::fs::rename(&source, Path::new(&original))?;
            }
        } else if source.exists() {
            crate::fs::rename(&source, &target)?;
        } else if Path::new(&entry.target).extension().is_some() {
            std::fs::File::create(&target).map_err(crate::fs::io_error(&target))?;
        } else {
            crate::fs::create_dir_all(&target)?;
        }

        if target.is_dir() {
            junction::create(&target, &source).map_err(crate::fs::io_error(&source))?;
        } else {
            std::fs::hard_link(&target, &source).map_err(|source_err| Error::Io {
                path: source.clone(),
                source: source_err,
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persist(source: &str, target: &str) -> Persist {
        Persist {
            source: source.into(),
            target: target.into(),
        }
    }

    #[test]
    fn link_moves_creates_and_survives_removal() {
        let root = std::env::temp_dir().join(format!("wpk-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("opt/p/1");
        let var = root.join("var/p");
        std::fs::create_dir_all(dir.join("conf")).unwrap();
        std::fs::write(dir.join("conf/a.ini"), "pkg").unwrap();
        std::fs::write(dir.join("settings.json"), "pkg").unwrap();
        std::fs::create_dir_all(&var).unwrap();
        std::fs::write(var.join("settings.json"), "user").unwrap();

        let entries = [
            persist("conf", "conf"),
            persist("settings.json", "settings.json"),
            persist("data", "data"),
            persist("cache\\x.db", "x.db"),
        ];
        link(&dir, &var, &entries).unwrap();

        assert_eq!(std::fs::read_to_string(var.join("conf/a.ini")).unwrap(), "pkg");
        assert_eq!(std::fs::read_to_string(dir.join("conf/a.ini")).unwrap(), "pkg");
        assert_eq!(std::fs::read_to_string(dir.join("settings.json")).unwrap(), "user");
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.original")).unwrap(), "pkg");
        assert!(var.join("data").is_dir() && dir.join("data").is_dir());
        assert!(var.join("x.db").is_file() && dir.join("cache/x.db").is_file());

        // Writes through the links land in var.
        std::fs::write(dir.join("data/new"), "n").unwrap();
        assert!(var.join("data/new").exists());

        // Removing the version dir keeps everything in var.
        crate::fs::remove_dir_all(&dir).unwrap();
        assert!(var.join("conf/a.ini").exists());
        assert!(var.join("data/new").exists());
        assert_eq!(std::fs::read_to_string(var.join("settings.json")).unwrap(), "user");

        // The next version links to the same data.
        std::fs::create_dir_all(&dir).unwrap();
        link(&dir, &var, &entries).unwrap();
        assert!(dir.join("data/new").exists());
    }
}
