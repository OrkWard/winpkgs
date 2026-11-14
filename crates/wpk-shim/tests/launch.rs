#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

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
fn arg0_replaces_the_program_name() {
    // .NET reads argv[0] from the command line, not the image path.
    let root = std::env::var("SystemRoot").unwrap();
    let powershell = format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe");
    let dir = shim_dir("arg0");
    let shim = make_shim(
        &dir,
        "cargo",
        &format!("path = {powershell}\narg0 = cargo\nargs = -NoProfile -Command [Environment]::GetCommandLineArgs()[0]\n"),
    );
    let out = Command::new(&shim).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "cargo");
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
