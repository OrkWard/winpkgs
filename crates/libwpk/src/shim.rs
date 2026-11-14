//! Writing shims: a copy of `wpk-shim.exe` or `wpk-shimw.exe` plus its
//! `.shim` file.

use std::path::{Path, PathBuf};

pub use wpk_shim::ShimFile;

use crate::fs::io_error;
use crate::{Error, Result};

/// The two shim binaries shims are copied from.
#[derive(Clone, Debug)]
pub struct ShimExes {
    pub console: PathBuf,
    pub gui: PathBuf,
}

impl ShimExes {
    /// The shims shipped next to the running wpk.
    pub fn beside_current_exe() -> Result<ShimExes> {
        let exe = std::env::current_exe().map_err(io_error(Path::new("wpk.exe")))?;
        let dir = exe.parent().expect("an exe has a parent dir");
        Ok(ShimExes {
            console: dir.join("wpk-shim.exe"),
            gui: dir.join("wpk-shimw.exe"),
        })
    }
}

/// Suffix of shim exes moved aside because they were running; GC deletes them.
pub const RETIRED_SUFFIX: &str = ".retired";

/// Create or update `dir/<name>.exe` and `dir/<name>.shim` to run
/// `shim.path`.
pub fn write(dir: &Path, name: &str, shim: &ShimFile, exes: &ShimExes) -> Result<()> {
    crate::fs::create_dir_all(dir)?;
    let source = if is_gui(Path::new(&shim.path)) {
        &exes.gui
    } else {
        &exes.console
    };
    let exe = dir.join(format!("{name}.exe"));
    let wanted = crate::fs::read(source)?;
    if std::fs::read(&exe).ok().as_deref() != Some(&wanted[..]) {
        retire(&exe)?;
        std::fs::write(&exe, &wanted).map_err(io_error(&exe))?;
    }
    crate::fs::write_atomic(&dir.join(format!("{name}.shim")), shim.render().as_bytes())
}

/// Remove `dir/<name>.exe` and its `.shim`. A running exe is moved aside.
pub fn remove(dir: &Path, name: &str) -> Result<()> {
    retire(&dir.join(format!("{name}.exe")))?;
    crate::fs::remove_file(&dir.join(format!("{name}.shim")))
}

pub fn read(dir: &Path, name: &str) -> Result<Option<ShimFile>> {
    let path = dir.join(format!("{name}.shim"));
    match std::fs::read_to_string(&path) {
        Ok(text) => ShimFile::parse(&text)
            .map(Some)
            .map_err(|message| Error::Corrupt { path, message }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io { path, source }),
    }
}

/// Delete `exe`, or rename it aside when it is running.
fn retire(exe: &Path) -> Result<()> {
    match std::fs::remove_file(exe) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            let stamp = jiff::Timestamp::now().as_millisecond();
            let mut aside = exe.as_os_str().to_owned();
            aside.push(format!(".{stamp}{RETIRED_SUFFIX}"));
            crate::fs::rename(exe, Path::new(&aside))
        }
    }
}

/// Whether `target` is a PE image for the Windows GUI subsystem.
pub fn is_gui(target: &Path) -> bool {
    const IMAGE_SUBSYSTEM_WINDOWS_GUI: u16 = 2;
    pe_subsystem(target) == Some(IMAGE_SUBSYSTEM_WINDOWS_GUI)
}

fn pe_subsystem(path: &Path) -> Option<u16> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(4096)
        .read_to_end(&mut head)
        .ok()?;
    let u32_at = |at: usize| Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?));
    if head.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3C)? as usize;
    if head.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    // The optional header follows the 20-byte file header; the subsystem
    // field is at offset 68 in both PE32 and PE32+.
    let at = pe + 4 + 20 + 68;
    Some(u16::from_le_bytes(head.get(at..at + 2)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsystems_of_system_binaries() {
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        assert!(!is_gui(&system.join("cmd.exe")));
        assert!(is_gui(&system.join("notepad.exe")));
        assert!(!is_gui(&system.join("does-not-exist.exe")));
    }

    #[test]
    fn write_then_remove() {
        let dir = crate::testutil::TempDir::new("shimw");
        let console = dir.join("c.exe");
        let gui = dir.join("g.exe");
        std::fs::write(&console, "console").unwrap();
        std::fs::write(&gui, "gui").unwrap();
        let exes = ShimExes { console, gui };
        let bin = dir.join("bin");
        let notepad =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\notepad.exe");
        let shim = ShimFile {
            path: notepad.display().to_string(),
            args: Some("-x".into()),
            dir: None,
            arg0: None,
        };
        write(&bin, "np", &shim, &exes).unwrap();
        assert_eq!(std::fs::read_to_string(bin.join("np.exe")).unwrap(), "gui");
        assert_eq!(read(&bin, "np").unwrap().unwrap(), shim);
        remove(&bin, "np").unwrap();
        assert!(!bin.join("np.exe").exists());
        assert!(read(&bin, "np").unwrap().is_none());
    }
}
