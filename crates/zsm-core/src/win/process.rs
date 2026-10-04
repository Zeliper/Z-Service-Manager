use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::OwnedHandle;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcess, ResumeThread, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::{io_err, owned, raw, Job};

static SPAWN_LOCK: Mutex<()> = Mutex::new(());

/// Serializes process creation so inheritable pipe ends never leak into a sibling spawn.
pub fn spawn_lock() -> MutexGuard<'static, ()> {
    SPAWN_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Process {
    handle: OwnedHandle,
    pid: u32,
}

impl Process {
    pub fn open(pid: u32) -> io::Result<Process> {
        let access = PROCESS_SYNCHRONIZE
            | PROCESS_QUERY_LIMITED_INFORMATION
            | PROCESS_SET_QUOTA
            | PROCESS_TERMINATE;
        let handle = unsafe { OpenProcess(access, false, pid) }.map_err(io_err)?;
        Ok(Process {
            handle: owned(handle),
            pid,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub(super) fn raw(&self) -> HANDLE {
        raw(&self.handle)
    }

    /// Waits for exit; returns the exit code, or `None` on timeout.
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<Option<u32>> {
        let ms = timeout.map_or(INFINITE, |t| t.as_millis().min(u32::MAX as u128 - 1) as u32);
        if unsafe { WaitForSingleObject(self.raw(), ms) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.raw(), &mut code) }.map_err(io_err)?;
        Ok(Some(code))
    }

    pub fn is_running(&self) -> bool {
        matches!(self.wait(Some(Duration::ZERO)), Ok(None))
    }

    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        unsafe { TerminateProcess(self.raw(), exit_code) }.map_err(io_err)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stdio {
    Inherit,
    Pipes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Normal,
    Hidden,
}

pub struct SpawnOptions<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a [(String, String)],
    pub stdio: Stdio,
    pub visibility: Visibility,
    pub new_process_group: bool,
    pub no_window: bool,
}

pub struct Spawned {
    pub process: Process,
    pub stdin: Option<File>,
    /// stdout and stderr merged into one pipe.
    pub stdout: Option<File>,
}

/// Creates the process suspended, assigns it to `job` (if any), then resumes it,
/// so no descendant can escape the job.
pub fn spawn(opts: &SpawnOptions, job: Option<&Job>) -> io::Result<Spawned> {
    let program = opts.program;
    let mut cmdline: Vec<u16> = std::iter::once(quote_arg(program))
        .chain(opts.args.iter().map(|a| quote_arg(a)))
        .collect::<Vec<_>>()
        .join(" ")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let env_block = environment_block(opts.env);
    let cwd = opts.cwd.map(super::wide);

    let _guard = spawn_lock();
    let pipes = match opts.stdio {
        Stdio::Pipes => Some(ChildPipes::new()?),
        Stdio::Inherit => None,
    };

    let mut flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;
    if opts.new_process_group {
        flags |= CREATE_NEW_PROCESS_GROUP;
    }
    if opts.no_window {
        flags |= CREATE_NO_WINDOW;
    }

    let mut si = STARTUPINFOEXW::default();
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    if opts.visibility == Visibility::Hidden {
        si.StartupInfo.dwFlags |= STARTF_USESHOWWINDOW;
        si.StartupInfo.wShowWindow = SW_HIDE.0 as u16;
    }

    let inherit_list: Vec<HANDLE>;
    let mut attr_buf: Vec<u8> = Vec::new();
    if let Some(p) = &pipes {
        si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = raw(&p.child_stdin);
        si.StartupInfo.hStdOutput = raw(&p.child_stdout);
        si.StartupInfo.hStdError = raw(&p.child_stdout);
        inherit_list = vec![raw(&p.child_stdin), raw(&p.child_stdout)];
        si.lpAttributeList = handle_list_attribute(&mut attr_buf, &inherit_list)?;
        flags |= EXTENDED_STARTUPINFO_PRESENT;
    }

    let mut pi = PROCESS_INFORMATION::default();
    let result = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(cmdline.as_mut_ptr())),
            None,
            None,
            pipes.is_some(),
            flags,
            Some(env_block.as_ptr() as *const _),
            cwd.as_ref().map_or(PCWSTR::null(), |c| PCWSTR(c.as_ptr())),
            &si.StartupInfo,
            &mut pi,
        )
    };
    if !si.lpAttributeList.0.is_null() {
        unsafe { DeleteProcThreadAttributeList(si.lpAttributeList) };
    }
    result.map_err(|e| {
        let err = io_err(e);
        io::Error::new(err.kind(), format!("{program}: {err}"))
    })?;

    let thread = owned(pi.hThread);
    let process = Process {
        handle: owned(pi.hProcess),
        pid: pi.dwProcessId,
    };
    if let Some(job) = job {
        if let Err(e) = job.assign(&process) {
            let _ = process.terminate(1);
            return Err(e);
        }
    }
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        let err = io::Error::last_os_error();
        let _ = process.terminate(1);
        return Err(err);
    }

    let (stdin, stdout) = match pipes {
        Some(p) => (
            Some(File::from(p.parent_stdin)),
            Some(File::from(p.parent_stdout)),
        ),
        None => (None, None),
    };
    Ok(Spawned {
        process,
        stdin,
        stdout,
    })
}

struct ChildPipes {
    child_stdin: OwnedHandle,
    child_stdout: OwnedHandle,
    parent_stdin: OwnedHandle,
    parent_stdout: OwnedHandle,
}

impl ChildPipes {
    fn new() -> io::Result<Self> {
        let (child_stdin, parent_stdin) = inheritable_pipe()?;
        let (parent_stdout, child_stdout) = inheritable_pipe()?;
        for h in [&parent_stdin, &parent_stdout] {
            unsafe { SetHandleInformation(raw(h), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
                .map_err(io_err)?;
        }
        Ok(Self {
            child_stdin,
            child_stdout,
            parent_stdin,
            parent_stdout,
        })
    }
}

fn inheritable_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, Some(&sa), 0) }.map_err(io_err)?;
    Ok((owned(read), owned(write)))
}

fn handle_list_attribute(
    buf: &mut Vec<u8>,
    handles: &[HANDLE],
) -> io::Result<LPPROC_THREAD_ATTRIBUTE_LIST> {
    let mut size = 0usize;
    unsafe {
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
    }
    buf.resize(size, 0);
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr() as *mut _);
    unsafe {
        InitializeProcThreadAttributeList(Some(list), 1, None, &mut size).map_err(io_err)?;
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            Some(handles.as_ptr() as *const _),
            std::mem::size_of_val(handles),
            None,
            None,
        )
        .map_err(io_err)?;
    }
    Ok(list)
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

fn environment_block(overrides: &[(String, String)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| {
            !overrides
                .iter()
                .any(|(o, _)| k.to_string_lossy().eq_ignore_ascii_case(o))
        })
        .collect();
    vars.extend(
        overrides
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v))),
    );
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(k.encode_wide());
        block.push('=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quoting_follows_msvc_rules() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg("with space"), "\"with space\"");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
    }
}
