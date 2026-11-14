#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn shim_dir(tag: &str) -> PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("wpk-shim-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Copy the console shim to `dir/name.exe` with the given `.shim` contents.
fn make_shim(dir: &Path, name: &str, shim: &str) -> PathBuf {
    let exe = dir.join(format!("{name}.exe"));
    std::fs::copy(env!("CARGO_BIN_EXE_wpk-shim"), &exe).unwrap();
    std::fs::write(dir.join(format!("{name}.shim")), shim).unwrap();
    exe
}

fn cmd_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap();
    format!("{root}\\System32\\cmd.exe")
}

/// Whether the exe at `path` stops running within `secs`: a running exe
/// cannot be deleted.
fn exits_within(path: &Path, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if std::fs::remove_file(path).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// A batch file in a new dir that starts `grand.exe`, a copy of cmd.exe,
/// with `line` after it.
fn batch_with_grandchild(tag: &str, line: &str) -> (PathBuf, PathBuf) {
    let dir = shim_dir(tag);
    let grand = dir.join("grand.exe");
    std::fs::copy(cmd_exe(), &grand).unwrap();
    let bat = dir.join("t.bat");
    let quoted = format!("\"{}\"", grand.display());
    std::fs::write(&bat, line.replace("GRAND", &quoted)).unwrap();
    let exe = make_shim(&dir, "t", &format!("path = {}\n", bat.display()));
    (exe, grand)
}

#[test]
fn killing_the_shim_ends_what_a_batch_file_runs() {
    let (exe, grand) = batch_with_grandchild(
        "bat-kill",
        "@GRAND /d /c echo started^& ping -n 30 127.0.0.1 ^>nul\r\n",
    );
    let mut shim = Command::new(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(shim.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "started");
    shim.kill().unwrap();
    shim.wait().unwrap();
    assert!(exits_within(&grand, 5), "grandchild outlived the shim");
}

#[test]
fn batch_files_can_leave_processes_behind() {
    let (exe, grand) = batch_with_grandchild(
        "bat-detach",
        "@start \"\" /b GRAND /d /c ping -n 5 127.0.0.1 >nul\r\n",
    );
    let status = Command::new(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    // A killed process may hold its exe for a moment while it goes.
    std::thread::sleep(Duration::from_secs(1));
    assert!(std::fs::remove_file(&grand).is_err(), "it was killed");
    assert!(exits_within(&grand, 10));
}

#[test]
fn returns_exit_code_and_passes_args() {
    let dir = shim_dir("exit");
    let exe = make_shim(&dir, "c", &format!("path = {}\nargs = /d /c\n", cmd_exe()));
    let out = Command::new(&exe)
        .args(["echo", "hi&&", "exit", "7"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
}

#[test]
fn runs_batch_files() {
    let dir = shim_dir("bat");
    let bat = dir.join("t.bat");
    std::fs::write(&bat, "@echo %1\r\n@exit /b 3\r\n").unwrap();
    let exe = make_shim(&dir, "t", &format!("path = {}\n", bat.display()));
    let out = Command::new(&exe).arg("arg1").output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "arg1");
}

#[test]
fn batch_arguments_cannot_inject_commands() {
    let dir = shim_dir("inject");
    let bat = dir.join("t.bat");
    std::fs::write(&bat, "@echo ARG=%1\r\n").unwrap();
    let exe = make_shim(&dir, "t", &format!("path = {}\n", bat.display()));
    let out = Command::new(&exe)
        .arg("a\"&echo INJECTED")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout
            .lines()
            .any(|l| l.contains("INJECTED") && !l.starts_with("ARG=")),
        "{stdout}"
    );
}

#[test]
fn reports_missing_shim_file() {
    let dir = shim_dir("missing");
    let exe = make_shim(&dir, "m", "");
    std::fs::remove_file(dir.join("m.shim")).unwrap();
    let out = Command::new(&exe).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("m.shim"));
}
