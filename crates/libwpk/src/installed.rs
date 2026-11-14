//! What is installed: `opt/<pkg>/current` and each version's receipt.

use serde::{Deserialize, Serialize};

use crate::manifest::Arch;
use crate::{Error, Layout, Result};

/// Written to `opt/<pkg>/<ver>/.wpk/receipt.json` when a version is
/// installed. Uninstall and upgrades act on what it lists, not on the
/// manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub version: String,
    pub bucket: String,
    /// Bucket commit the manifest came from.
    pub commit: Option<String>,
    pub arch: Arch,
    pub installed_at: jiff::Timestamp,
    /// Installed on request rather than as a dependency.
    pub explicit: bool,
    /// Names of the shims in `bin`.
    #[serde(default)]
    pub bins: Vec<String>,
    /// Names of the shims in `opt/wpk/launchers/<pkg>`.
    #[serde(default)]
    pub launchers: Vec<String>,
    /// Shortcut files, relative to the Start menu folder.
    #[serde(default)]
    pub shortcuts: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub name: String,
    pub receipt: Receipt,
    pub held: bool,
}

/// The active version of `name`, if installed.
pub fn current(layout: &Layout, name: &str) -> Result<Option<String>> {
    match std::fs::read_to_string(layout.current(name)) {
        Ok(text) => Ok(Some(text.trim().to_owned()).filter(|v| !v.is_empty())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: layout.current(name),
            source,
        }),
    }
}

pub fn set_current(layout: &Layout, name: &str, version: &str) -> Result<()> {
    crate::fs::write_atomic(&layout.current(name), version.as_bytes())
}

pub fn receipt(layout: &Layout, name: &str, version: &str) -> Result<Receipt> {
    let path = layout.records(name, version).join("receipt.json");
    let text = crate::fs::read(&path)?;
    serde_json::from_slice(&text).map_err(|e| Error::Corrupt {
        path,
        message: e.to_string(),
    })
}

pub fn write_receipt(layout: &Layout, name: &str, receipt: &Receipt) -> Result<()> {
    let dir = layout.records(name, &receipt.version);
    crate::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(receipt).expect("receipt serializes");
    crate::fs::write_atomic(&dir.join("receipt.json"), &json)
}

pub fn get(layout: &Layout, name: &str) -> Result<Option<Installed>> {
    let Some(version) = current(layout, name)? else {
        return Ok(None);
    };
    Ok(Some(Installed {
        name: name.to_owned(),
        receipt: receipt(layout, name, &version)?,
        held: layout.held(name).exists(),
    }))
}

/// Every installed package, sorted by name. Broken installs are reported
/// through `broken` and left out.
pub fn list(layout: &Layout, mut broken: impl FnMut(&str, Error)) -> Result<Vec<Installed>> {
    let opt = layout.opt();
    let entries = match std::fs::read_dir(&opt) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Io { path: opt, source }),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        match get(layout, &name) {
            Ok(Some(installed)) => out.push(installed),
            Ok(None) => {}
            Err(e) => broken(&name, e),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn set_held(layout: &Layout, name: &str, held: bool) -> Result<()> {
    if current(layout, name)?.is_none() {
        return Err(Error::NotInstalled(name.to_owned()));
    }
    let path = layout.held(name);
    if held {
        crate::fs::create_dir_all(path.parent().expect("held has a parent"))?;
        crate::fs::write_atomic(&path, b"")
    } else {
        crate::fs::remove_file(&path)
    }
}
