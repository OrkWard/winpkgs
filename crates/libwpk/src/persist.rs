//! `persist`: data kept in `var/<pkg>` and linked into each version, and
//! `persist_copy`: data copied from version to version.

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

/// Copy each `persist_copy` path into the new version dir `dir`, from the
/// installed version dir `from` when there is one, else from `var_dir`,
/// where [`copy_back`] saves it on uninstall. A path found in neither keeps
/// whatever the package ships.
pub fn copy_forward(dir: &Path, from: Option<&Path>, var_dir: &Path, entries: &[String]) -> Result<()> {
    for entry in entries {
        let source = from
            .map(|from| from.join(entry))
            .filter(|p| p.exists())
            .or_else(|| Some(var_dir.join(entry)).filter(|p| p.exists()));
        if let Some(source) = source {
            replace_with_copy(&source, &dir.join(entry))?;
        }
    }
    Ok(())
}

/// Save each `persist_copy` path of the version dir `dir` to `var_dir`.
pub fn copy_back(dir: &Path, var_dir: &Path, entries: &[String]) -> Result<()> {
    for entry in entries {
        let source = dir.join(entry);
        if source.exists() {
            replace_with_copy(&source, &var_dir.join(entry))?;
        }
    }
    Ok(())
}

fn replace_with_copy(source: &Path, dest: &Path) -> Result<()> {
    match std::fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_dir() => crate::fs::remove_dir_all(dest)?,
        Ok(_) => crate::fs::remove_file(dest)?,
        Err(_) => {}
    }
    if let Some(parent) = dest.parent() {
        crate::fs::create_dir_all(parent)?;
    }
    copy_recursive(source, dest)
}

fn copy_recursive(source: &Path, dest: &Path) -> Result<()> {
    if source.is_dir() {
        crate::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(source).map_err(crate::fs::io_error(source))? {
            let entry = entry.map_err(crate::fs::io_error(source))?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, dest).map(|_| ()).map_err(crate::fs::io_error(source))
    }
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
        let root = crate::testutil::TempDir::new("persist");
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

        assert_eq!(
            std::fs::read_to_string(var.join("conf/a.ini")).unwrap(),
            "pkg"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("conf/a.ini")).unwrap(),
            "pkg"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json")).unwrap(),
            "user"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.json.original")).unwrap(),
            "pkg"
        );
        assert!(var.join("data").is_dir() && dir.join("data").is_dir());
        assert!(var.join("x.db").is_file() && dir.join("cache/x.db").is_file());

        // Writes through the links land in var.
        std::fs::write(dir.join("data/new"), "n").unwrap();
        assert!(var.join("data/new").exists());

        // Removing the version dir keeps everything in var.
        crate::fs::remove_dir_all(&dir).unwrap();
        assert!(var.join("conf/a.ini").exists());
        assert!(var.join("data/new").exists());
        assert_eq!(
            std::fs::read_to_string(var.join("settings.json")).unwrap(),
            "user"
        );

        // The next version links to the same data.
        std::fs::create_dir_all(&dir).unwrap();
        link(&dir, &var, &entries).unwrap();
        assert!(dir.join("data/new").exists());
    }

    #[test]
    fn copy_forward_prefers_installed_version_then_var() {
        let root = crate::testutil::TempDir::new("persist-copy");
        let (v1, v2, var) = (root.join("1"), root.join("2"), root.join("var"));
        std::fs::create_dir_all(v1.join("d")).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("a.ini"), "user").unwrap();
        std::fs::write(v1.join("d/x"), "x").unwrap();
        std::fs::write(v2.join("a.ini"), "shipped").unwrap();
        std::fs::write(v2.join("b.ini"), "shipped").unwrap();
        let entries = ["a.ini".to_owned(), "b.ini".to_owned(), "d".to_owned(), "none".to_owned()];

        copy_forward(&v2, Some(&v1), &var, &entries).unwrap();
        assert_eq!(std::fs::read_to_string(v2.join("a.ini")).unwrap(), "user");
        assert_eq!(std::fs::read_to_string(v2.join("b.ini")).unwrap(), "shipped");
        assert_eq!(std::fs::read_to_string(v2.join("d/x")).unwrap(), "x");
        assert!(!v2.join("none").exists());

        // Uninstall saves to var, a later install restores from there.
        copy_back(&v2, &var, &entries).unwrap();
        let v3 = root.join("3");
        std::fs::create_dir_all(&v3).unwrap();
        copy_forward(&v3, None, &var, &entries).unwrap();
        assert_eq!(std::fs::read_to_string(v3.join("a.ini")).unwrap(), "user");
        assert_eq!(std::fs::read_to_string(v3.join("d/x")).unwrap(), "x");
    }
}
