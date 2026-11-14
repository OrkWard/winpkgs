//! Garbage collection: removing versions no longer current.
//!
//! Upgrades only switch `current` and the shims, so the old version may keep
//! running. GC deletes it later, skipping anything still in use.

use std::path::{Path, PathBuf};

use crate::{Error, Layout, Result, Wpk, installed, shim};

/// What a collection removed and what it had to leave.
#[derive(Debug, Default)]
pub struct Report {
    pub removed: Vec<PathBuf>,
    /// In use, or could not be deleted; tried again next time.
    pub kept: Vec<(PathBuf, String)>,
}

/// Run [`run`] when the last run is older than `gc_interval_days`.
pub fn run_if_due(wpk: &Wpk) -> Result<Option<Report>> {
    let stamp = wpk.layout.gc_stamp();
    let interval = std::time::Duration::from_secs(u64::from(wpk.config.gc_interval_days) * 86400);
    let due = match std::fs::metadata(&stamp).and_then(|m| m.modified()) {
        Ok(last) => last.elapsed().map_or(true, |age| age >= interval),
        Err(_) => true,
    };
    if !due {
        return Ok(None);
    }
    run(&wpk.layout).map(Some)
}

/// Collect every package, then the retired shim exes, and stamp the time.
pub fn run(layout: &Layout) -> Result<Report> {
    let mut report = Report::default();
    let opt = layout.opt();
    if let Ok(entries) = std::fs::read_dir(&opt) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                collect_into(layout, &name, &mut report)?;
            }
        }
    }
    let mut shim_dirs = vec![layout.bin()];
    if let Ok(entries) = std::fs::read_dir(layout.state().join("launchers")) {
        shim_dirs.extend(entries.flatten().map(|e| e.path()));
    }
    for dir in shim_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.to_string_lossy().ends_with(shim::RETIRED_SUFFIX) {
                match std::fs::remove_file(&path) {
                    Ok(()) => report.removed.push(path),
                    Err(e) => report.kept.push((path, e.to_string())),
                }
            }
        }
    }
    crate::fs::create_dir_all(&layout.state())?;
    crate::fs::write_atomic(&layout.gc_stamp(), jiff::Timestamp::now().to_string().as_bytes())?;
    Ok(report)
}

/// Remove the versions of `name` other than the current one. With no
/// current version, the package dir goes too once empty.
pub fn collect_package(layout: &Layout, name: &str) -> Result<Report> {
    let mut report = Report::default();
    collect_into(layout, name, &mut report)?;
    match report.kept.first() {
        Some((path, why)) => Err(Error::Install {
            package: name.to_owned(),
            message: format!("{} kept: {why}", path.display()),
        }),
        None => Ok(report),
    }
}

fn collect_into(layout: &Layout, name: &str, report: &mut Report) -> Result<()> {
    let current = installed::current(layout, name)?;
    let keep = current
        .as_deref()
        .map(|v| layout.version(name, v).to_string_lossy().to_lowercase());
    let package = layout.package(name);
    let Ok(entries) = std::fs::read_dir(&package) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Paths are case-insensitive on Windows.
        let is_current = keep.as_deref() == Some(path.to_string_lossy().to_lowercase().as_str());
        if !entry.file_type().is_ok_and(|t| t.is_dir()) || is_current {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let is_version = path.join(".wpk").join("receipt.json").exists();
        let is_leftover = file_name.ends_with(".tmp") || file_name.starts_with(".wpk-gc-");
        // opt/wpk also holds wpk's state; only version dirs are collected.
        if !is_version && !is_leftover {
            continue;
        }
        match remove_version(&package, &path, &file_name) {
            Ok(()) => report.removed.push(path),
            Err(why) => report.kept.push((path, why)),
        }
    }
    if current.is_none() {
        let _ = std::fs::remove_dir(&package);
    }
    Ok(())
}

/// Delete a version dir unless something in it is in use.
fn remove_version(package: &Path, path: &Path, file_name: &str) -> std::result::Result<(), String> {
    if let Some(busy) = find_busy_image(path) {
        return Err(format!("{} is running", busy.display()));
    }
    // Renaming first fails while files inside are open, so a version in use
    // is not left half deleted.
    let doomed = if file_name.starts_with(".wpk-gc-") {
        path.to_owned()
    } else {
        let doomed = package.join(format!(".wpk-gc-{file_name}"));
        std::fs::rename(path, &doomed).map_err(|e| format!("in use: {e}"))?;
        doomed
    };
    remove_dir_all::remove_dir_all(&doomed).map_err(|e| e.to_string())
}

/// Extensions of files a running process maps and keeps locked.
const IMAGE_EXTENSIONS: [&str; 5] = ["exe", "dll", "com", "pyd", "node"];

/// The first PE image under `dir` that a process has loaded. Links, such as
/// persisted data, are not followed.
pub fn find_busy_image(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if let Some(busy) = find_busy_image(&path) {
                return Some(busy);
            }
        } else if is_image(&path) && is_busy(&path) {
            return Some(path);
        }
    }
    None
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A loaded image refuses write access; opening for write without sharing
/// tells whether one is loaded. Nothing is written.
#[cfg(windows)]
fn is_busy(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    match std::fs::OpenOptions::new().write(true).share_mode(0).open(path) {
        Ok(_) => false,
        Err(e) => matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)),
    }
}

#[cfg(not(windows))]
fn is_busy(_: &Path) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn keeps_current_running_and_state() {
        let root = std::env::temp_dir().join(format!("wpk-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let layout = Layout::new(&root, root.join("cache"));
        let version = |v: &str| {
            let dir = layout.version("p", v);
            std::fs::create_dir_all(dir.join(".wpk")).unwrap();
            std::fs::write(dir.join(".wpk/receipt.json"), "{}").unwrap();
            dir
        };
        let old = version("1");
        let busy = version("2");
        let cur = version("3");
        std::fs::create_dir_all(layout.package("p").join("4.tmp")).unwrap();
        installed::set_current(&layout, "p", "3").unwrap();
        std::fs::create_dir_all(layout.buckets().join("main")).unwrap();
        std::fs::create_dir_all(layout.version("wpk", "0.1").join(".wpk")).unwrap();
        installed::set_current(&layout, "wpk", "0.1").unwrap();

        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        let exe = busy.join("ping.exe");
        std::fs::copy(system.join("PING.EXE"), &exe).unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let report = run(&layout).unwrap();
        assert!(!old.exists());
        assert!(!layout.package("p").join("4.tmp").exists());
        assert!(busy.exists() && cur.exists());
        assert_eq!(report.kept.len(), 1);
        assert!(layout.buckets().join("main").exists());
        assert!(layout.gc_stamp().exists());

        child.kill().unwrap();
        child.wait().unwrap();
        run(&layout).unwrap();
        assert!(!busy.exists() && cur.exists());
    }
}
