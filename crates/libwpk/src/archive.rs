//! Turning downloads into files in the install dir.
//!
//! zip is built in. msi goes through `msiexec /a`, innosetup through the
//! `innounp` package and every other archive through the `7zip` package.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fs::io_error;
use crate::manifest::{Download, Extract};
use crate::{Error, Result};

/// External extractors, from the installed `7zip` and `innounp` packages.
#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub seven_zip: Option<PathBuf>,
    pub innounp: Option<PathBuf>,
}

/// Put `file`, a copy of the download named `download.file_name`, into
/// `dir`: extracted when it is an archive, as is otherwise. The archive
/// itself is removed after extraction.
pub fn unpack(file: &Path, download: &Download, dir: &Path, tools: &Tools) -> Result<()> {
    if download.extract == Extract::None {
        let dest = dir.join(&download.file_name);
        if file != dest {
            crate::fs::rename(file, &dest)?;
        }
        return Ok(());
    }

    let tmp = dir.join(format!(".wpk-extract-{}", download.file_name));
    crate::fs::remove_dir_all(&tmp)?;
    crate::fs::create_dir_all(&tmp)?;
    let result = (|| {
        let root = match download.extract {
            Extract::None => unreachable!(),
            Extract::Zip => {
                extract_zip(file, &tmp)?;
                tmp.clone()
            }
            Extract::Msi => extract_msi(file, &tmp)?,
            Extract::Inno => {
                let tool = tools.innounp.as_deref().ok_or(Error::MissingTool("innounp"))?;
                extract_inno(tool, file, &tmp, download.extract_dir.as_deref())?;
                tmp.clone()
            }
            Extract::SevenZip => {
                let tool = tools.seven_zip.as_deref().ok_or(Error::MissingTool("7zip"))?;
                extract_7z(tool, file, &tmp)?;
                tmp.clone()
            }
        };
        let source = match (&download.extract_dir, download.extract) {
            (Some(sub), kind) if kind != Extract::Inno => root.join(sub),
            _ => root,
        };
        if !source.is_dir() {
            return Err(Error::Extract {
                file: download.file_name.clone(),
                message: format!(
                    "extract_dir '{}' not found",
                    download.extract_dir.as_deref().unwrap_or_default()
                ),
            });
        }
        let dest = match &download.extract_to {
            Some(sub) => dir.join(sub),
            None => dir.to_owned(),
        };
        merge_dir(&source, &dest)
    })();
    crate::fs::remove_dir_all(&tmp)?;
    result?;
    crate::fs::remove_file(file)
}

fn extract_zip(file: &Path, dest: &Path) -> Result<()> {
    let err = |message: String| Error::Extract {
        file: file.display().to_string(),
        message,
    };
    let reader = std::fs::File::open(file).map_err(io_error(file))?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| err(e.to_string()))?;
    archive.extract(dest).map_err(|e| err(e.to_string()))
}

/// Administrative install into `tmp`, returning the dir holding the files.
fn extract_msi(file: &Path, tmp: &Path) -> Result<PathBuf> {
    let target = tmp.join("SourceDir");
    let log = tmp.join("msi.log");
    let msiexec = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("msiexec.exe"))
        .unwrap_or_else(|| PathBuf::from("msiexec.exe"));
    let mut command = Command::new(msiexec);
    command.arg("/a").arg(file).arg("/qn");
    // msiexec parses PROPERTY="value" itself; std's quoting would wrap the
    // whole argument.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(format!("TARGETDIR=\"{}\"", target.display()));
        command.raw_arg(format!("/lwe \"{}\"", log.display()));
    }
    let status = command.status().map_err(|e| Error::Extract {
        file: file.display().to_string(),
        message: format!("cannot run msiexec: {e}"),
    })?;
    if !status.success() {
        let tail = read_log_tail(&log);
        return Err(Error::Extract {
            file: file.display().to_string(),
            message: format!("msiexec failed ({status}){tail}"),
        });
    }
    let _ = std::fs::remove_file(&log);
    // msiexec leaves a copy of the package in the target.
    if let Some(name) = file.file_name() {
        let _ = std::fs::remove_file(target.join(name));
    }
    Ok(target)
}

/// The end of an msiexec log, which is UTF-16 LE.
fn read_log_tail(log: &Path) -> String {
    let Ok(bytes) = std::fs::read(log) else {
        return String::new();
    };
    let text = match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(utf16) => {
            let units: Vec<u16> = utf16.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(&bytes).into_owned(),
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(8)..].join("\n");
    if tail.is_empty() { tail } else { format!(":\n{tail}") }
}

fn extract_inno(tool: &Path, file: &Path, dest: &Path, extract_dir: Option<&str>) -> Result<()> {
    let filter = match extract_dir {
        Some(dir) if dir.starts_with('{') => format!("-c{dir}"),
        Some(dir) => format!("-c{{app}}\\{dir}"),
        None => "-c{app}".to_owned(),
    };
    let mut dest_arg = std::ffi::OsString::from("-d");
    dest_arg.push(dest);
    run_tool(
        Command::new(tool).arg("-x").arg(dest_arg).arg(file).arg("-y").arg(filter),
        file,
    )
}

fn extract_7z(tool: &Path, file: &Path, dest: &Path) -> Result<()> {
    let seven = |archive: &Path, out: &Path| {
        let mut out_arg = std::ffi::OsString::from("-o");
        out_arg.push(out);
        run_tool(
            Command::new(tool)
                .arg("x")
                .arg(archive)
                .arg(out_arg)
                .args(["-xr!*.nsis", "-y", "-bd"]),
            archive,
        )
    };
    seven(file, dest)?;
    // `.tar.gz` and friends unpack to a lone `.tar`.
    let name = file.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let mut exts = name.rsplit('.');
    let ext = exts.next().unwrap_or_default();
    let tarball = exts.next() == Some("tar")
        || ext == "tzst"
        || matches!(ext.as_bytes(), [b't', b'a' | b'b' | b'g' | b'p' | b'x', b'z', rest @ ..] if rest.is_empty() || rest == b"2");
    if tarball {
        let entries: Vec<_> = std::fs::read_dir(dest)
            .map_err(io_error(dest))?
            .flatten()
            .collect();
        if let [entry] = entries.as_slice() {
            let inner = entry.path();
            if inner.extension().is_some_and(|e| e.eq_ignore_ascii_case("tar")) {
                seven(&inner, dest)?;
                crate::fs::remove_file(&inner)?;
            }
        }
    }
    Ok(())
}

fn run_tool(command: &mut Command, file: &Path) -> Result<()> {
    let output = command.output().map_err(|e| Error::Extract {
        file: file.display().to_string(),
        message: format!("cannot run {}: {e}", command.get_program().to_string_lossy()),
    })?;
    if output.status.success() {
        return Ok(());
    }
    let mut message = format!("{} failed ({})", command.get_program().to_string_lossy(), output.status);
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        let text = text.trim();
        if !text.is_empty() {
            message.push('\n');
            message.push_str(text);
        }
    }
    Err(Error::Extract {
        file: file.display().to_string(),
        message,
    })
}

/// Move the contents of `src` into `dest`, merging directories and
/// replacing files.
pub fn merge_dir(src: &Path, dest: &Path) -> Result<()> {
    crate::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src).map_err(io_error(src))? {
        let entry = entry.map_err(io_error(src))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let from_is_dir = entry.file_type().map_err(io_error(&from))?.is_dir();
        match std::fs::symlink_metadata(&to) {
            Ok(meta) if meta.is_dir() && from_is_dir => merge_dir(&from, &to)?,
            Ok(meta) => {
                if meta.is_dir() {
                    crate::fs::remove_dir_all(&to)?;
                } else {
                    crate::fs::remove_file(&to)?;
                }
                crate::fs::rename(&from, &to)?;
            }
            Err(_) => crate::fs::rename(&from, &to)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wpk-archive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn download(name: &str, extract_dir: Option<&str>, extract_to: Option<&str>) -> Download {
        Download {
            url: format!("https://h/{name}"),
            file_name: name.to_owned(),
            hash: None,
            extract: Extract::of(name, false),
            extract_dir: extract_dir.map(Into::into),
            extract_to: extract_to.map(Into::into),
        }
    }

    fn write_zip(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, content) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn zip_with_extract_dir_and_to() {
        let dir = temp("zip");
        let file = dir.join("a.zip");
        write_zip(&file, &[("top/bin/x.exe", "x"), ("top/readme", "r")]);
        unpack(&file, &download("a.zip", Some("top"), Some("sub")), &dir, &Tools::default()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("sub/bin/x.exe")).unwrap(), "x");
        assert!(!file.exists());
        assert!(!dir.join(".wpk-extract-a.zip").exists());
    }

    #[test]
    fn missing_extract_dir_fails_cleanly() {
        let dir = temp("missing");
        let file = dir.join("a.zip");
        write_zip(&file, &[("x", "x")]);
        let err = unpack(&file, &download("a.zip", Some("nope"), None), &dir, &Tools::default());
        assert!(matches!(err, Err(Error::Extract { .. })));
        assert!(!dir.join(".wpk-extract-a.zip").exists());
    }

    #[test]
    fn plain_files_are_moved() {
        let dir = temp("plain");
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let file = stage.join("tool.exe");
        std::fs::write(&file, "t").unwrap();
        unpack(&file, &download("tool.exe", None, None), &dir, &Tools::default()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("tool.exe")).unwrap(), "t");
    }

    #[test]
    fn seven_zip_needs_the_tool() {
        let dir = temp("no7z");
        let file = dir.join("a.7z");
        std::fs::write(&file, "x").unwrap();
        let err = unpack(&file, &download("a.7z", None, None), &dir, &Tools::default());
        assert!(matches!(err, Err(Error::MissingTool("7zip"))));
    }

    /// Uses Scoop's 7-Zip when present.
    #[test]
    fn tar_gz_through_7z() {
        let Some(seven) = std::env::home_dir()
            .map(|h| h.join("scoop/apps/7zip/current/7z.exe"))
            .filter(|p| p.exists())
        else {
            return;
        };
        let dir = temp("tgz");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("pkg-1")).unwrap();
        std::fs::write(src.join("pkg-1/f.txt"), "f").unwrap();
        let tar = dir.join("a.tar");
        let tgz = dir.join("a.tar.gz");
        let status = Command::new(&seven)
            .args(["a", "-ttar", "-bso0"])
            .arg(&tar)
            .arg(src.join("pkg-1"))
            .status()
            .unwrap();
        assert!(status.success());
        let status = Command::new(&seven).args(["a", "-tgzip", "-bso0"]).arg(&tgz).arg(&tar).status().unwrap();
        assert!(status.success());
        std::fs::remove_file(&tar).unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let tools = Tools {
            seven_zip: Some(seven),
            innounp: None,
        };
        unpack(&tgz, &download("a.tar.gz", Some("pkg-1"), None), &out, &tools).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("f.txt")).unwrap(), "f");
    }
}
