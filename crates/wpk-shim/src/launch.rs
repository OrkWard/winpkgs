//! Starting the target process.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ELEVATION_REQUIRED, HANDLE, LocalFree};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, GetCurrentProcess, GetExitCodeProcess, GetStartupInfoW,
    INFINITE, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    CommandLineToArgvW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL,
};

use crate::{ShimFile, Variant, skip_program_name};

const FILE_SHARE_ALL: u32 = 1 | 2 | 4;

pub fn run(variant: Variant) -> Result<i32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate itself: {e}"))?;
    let shim_path = exe.with_extension("shim");
    let text = read_shared(&shim_path).map_err(|e| format!("{}: {e}", shim_path.display()))?;
    let shim = ShimFile::parse(&text).map_err(|e| format!("{}: {e}", shim_path.display()))?;
    let rest = own_args();

    if variant == Variant::Console {
        // The target shares our console and gets Ctrl+C itself; we only wait.
        unsafe { SetConsoleCtrlHandler(Some(ignore_ctrl), 1) };
    }
    if is_batch(&shim.path) {
        return run_batch(&shim, rest);
    }

    // Pass on how we were started: window state, and the AppUserModelID or
    // link name the shell set when starting us from a shortcut.
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = size_of::<STARTUPINFOW>() as u32;
    unsafe { GetStartupInfoW(&mut si) };
    si.lpReserved = null_mut();
    si.cbReserved2 = 0;
    si.lpReserved2 = null_mut();

    let mut line = shim.command_line(rest);
    line.push(0);
    let dir: Option<Vec<u16>> = shim.dir.as_deref().map(|d| wide(OsStr::new(d)));
    let dir_ptr = dir.as_ref().map_or(null(), |d| d.as_ptr());
    let flags = match variant {
        Variant::Console => CREATE_SUSPENDED,
        Variant::Gui => 0,
    };
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            flags,
            null(),
            dir_ptr,
            &si,
            &mut pi,
        )
    };
    if created == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED as i32) {
            return run_elevated(variant, &shim, rest);
        }
        return Err(format!("cannot start {}: {error}", shim.path));
    }

    if variant == Variant::Gui {
        unsafe {
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
        return Ok(0);
    }
    // Resumed only now so the target cannot start children outside the job.
    unsafe {
        kill_with_us(pi.hProcess);
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
    }
    Ok(wait(pi.hProcess))
}

pub fn report(variant: Variant, message: &str) {
    match variant {
        Variant::Console => eprintln!("{message}"),
        Variant::Gui => {
            let text = wide(OsStr::new(message));
            let caption = wide(OsStr::new("wpk-shim"));
            unsafe {
                MessageBoxW(
                    null_mut(),
                    text.as_ptr(),
                    caption.as_ptr(),
                    MB_OK | MB_ICONERROR,
                )
            };
        }
    }
}

/// Read the `.shim` file without blocking its replacement during upgrades.
fn read_shared(path: &std::path::Path) -> io::Result<String> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_ALL)
        .open(path)?;
    let mut text = String::new();
    io::Read::read_to_string(&mut file, &mut text)?;
    Ok(text)
}

fn own_args() -> &'static [u16] {
    unsafe {
        let line = GetCommandLineW();
        let mut len = 0;
        while *line.add(len) != 0 {
            len += 1;
        }
        skip_program_name(std::slice::from_raw_parts(line, len))
    }
}

unsafe extern "system" fn ignore_ctrl(_ctrl_type: u32) -> i32 {
    1
}

/// Put the process in a job that ends it when the shim dies.
unsafe fn kill_with_us(process: HANDLE) {
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return;
        }
        set_job_limits(
            job,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
        );
        AssignProcessToJobObject(job, process);
        // The handle stays open until we exit, which is what closes the job.
    }
}

/// Put the shim in a job that ends every process started from here on,
/// and their children, when the shim dies. Null when that fails.
unsafe fn kill_all_with_us() -> HANDLE {
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return job;
        }
        set_job_limits(job, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            CloseHandle(job);
            return null_mut();
        }
        job
    }
}

unsafe fn set_job_limits(job: HANDLE, flags: u32) {
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = flags;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
    }
}

fn wait(process: HANDLE) -> i32 {
    let mut code = 0u32;
    unsafe {
        WaitForSingleObject(process, INFINITE);
        GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
    }
    code as i32
}

/// Targets whose manifest requires elevation can only start through the shell.
fn run_elevated(variant: Variant, shim: &ShimFile, rest: &[u16]) -> Result<i32, String> {
    let verb = wide(OsStr::new("runas"));
    let file = wide(OsStr::new(&shim.path));
    let dir = shim.dir.as_deref().map(|d| wide(OsStr::new(d)));
    let mut params: Vec<u16> = shim.args.as_deref().unwrap_or("").encode_utf16().collect();
    if !rest.is_empty() {
        if !params.is_empty() {
            params.push(b' ' as u16);
        }
        params.extend_from_slice(rest);
    }
    params.push(0);

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.lpDirectory = dir.as_ref().map_or(null(), |d| d.as_ptr());
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(format!(
            "cannot start {}: {}",
            shim.path,
            io::Error::last_os_error()
        ));
    }
    if info.hProcess.is_null() {
        return Ok(0);
    }
    match variant {
        Variant::Console => Ok(wait(info.hProcess)),
        Variant::Gui => {
            unsafe { CloseHandle(info.hProcess) };
            Ok(0)
        }
    }
}

fn is_batch(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".bat") || lower.ends_with(".cmd")
}

/// cmd.exe parses a batch file's command line by its own rules, not the C
/// runtime's our callers quoted for, so the arguments are split and handed
/// to std, which quotes them for cmd or refuses ones it cannot make safe.
///
/// What the batch file runs is the real target, so the whole tree joins the
/// job, by inheriting ours from the start.
fn run_batch(shim: &ShimFile, rest: &[u16]) -> Result<i32, String> {
    let mut args = shim.args.as_deref().map(split_args).unwrap_or_default();
    args.extend(split_args(&String::from_utf16_lossy(rest)));
    let mut command = std::process::Command::new(&shim.path);
    command.args(&args);
    if let Some(dir) = &shim.dir {
        command.current_dir(dir);
    }
    let job = unsafe { kill_all_with_us() };
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", shim.path))?;
    if job.is_null() {
        unsafe { kill_with_us(child.as_raw_handle() as HANDLE) };
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !job.is_null() {
        // What the batch file left running, e.g. with `start`, was meant to
        // outlive it.
        unsafe { set_job_limits(job, 0) };
    }
    Ok(status.code().unwrap_or(1))
}

/// Split a command line the way the C runtime does.
fn split_args(line: &str) -> Vec<String> {
    if line.trim().is_empty() {
        return Vec::new();
    }
    // CommandLineToArgvW treats the first word as the program name.
    let full = wide(OsStr::new(&format!("x {line}")));
    let mut count = 0;
    unsafe {
        let argv = CommandLineToArgvW(full.as_ptr(), &mut count);
        if argv.is_null() {
            return Vec::new();
        }
        let args = (1..count as usize)
            .map(|i| {
                let p = *argv.add(i);
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
            })
            .collect();
        LocalFree(argv as _);
        args
    }
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
