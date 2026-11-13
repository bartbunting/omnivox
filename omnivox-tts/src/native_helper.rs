//! Private Windows launch injection for a guarded worker. Ordinary helper
//! startup retains std::process::Command and its existing pipe behavior.
use crate::engine_configuration::LaunchEnvironment;
use std::ffi::{c_void, OsString};
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::OnceLock;

pub type Launcher = fn(&str, &Path, &[OsString], &LaunchEnvironment) -> io::Result<Child>;
static LAUNCHER: OnceLock<Launcher> = OnceLock::new();

/// Install only at the private worker entry before engine threads start.
pub fn install(launcher: Launcher) -> io::Result<()> {
    LAUNCHER
        .set(launcher)
        .map_err(|_| io::Error::other("native helper launcher already installed"))
}

enum Process {
    Ordinary(std::process::Child),
    Guarded(OwnedHandle),
}

pub struct Child {
    process: Process,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    id: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
    fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
    fn GetProcessId(handle: *mut c_void) -> u32;
    fn TerminateProcess(handle: *mut c_void, code: u32) -> i32;
}

impl Child {
    /// Handles come from one complete guard handoff. The actual process handle
    /// retains SYNCHRONIZE, QUERY_LIMITED_INFORMATION and TERMINATE authority.
    pub fn guarded(
        process: OwnedHandle,
        input: OwnedHandle,
        output: OwnedHandle,
    ) -> io::Result<Self> {
        // SAFETY: borrowing the supplied live handle; failure does not adopt a PID.
        let id = unsafe { GetProcessId(process.as_raw_handle()) };
        if id == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            process: Process::Guarded(process),
            stdin: Some(input.into()),
            stdout: Some(output.into()),
            id,
        })
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.process {
            Process::Ordinary(child) => child.try_wait(),
            Process::Guarded(handle) => {
                // SAFETY: this retained handle identifies the actual process.
                match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
                    258 => Ok(None),
                    0 => {
                        let mut status = 0;
                        // SAFETY: actual process completion precedes reading its exit code.
                        if unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut status) } == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(Some(ExitStatus::from_raw(status)))
                    }
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }
    }
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.process {
            Process::Ordinary(child) => child.kill(),
            Process::Guarded(handle) => {
                // SAFETY: the guard transferred cancellation for this exact helper.
                if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }
    }
}

pub(crate) fn spawn(
    engine: &str,
    program: &Path,
    arguments: &[OsString],
    environment: &LaunchEnvironment,
) -> io::Result<Child> {
    if let Some(launch) = LAUNCHER.get() {
        return launch(engine, program, arguments, environment);
    }
    let mut command = Command::new(program);
    environment.apply(&mut command);
    let mut child = command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    Ok(Child {
        id: child.id(),
        stdin: child.stdin.take(),
        stdout: child.stdout.take(),
        process: Process::Ordinary(child),
    })
}
