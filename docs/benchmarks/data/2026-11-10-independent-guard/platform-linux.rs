//! Linux validation lives in a dedicated subreaper, never in a speech worker.
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

unsafe extern "C" {
    fn prctl(option: i32, ...) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn setrlimit(resource: i32, limit: *const Limit) -> i32;
}
#[repr(C)]
struct Limit {
    current: usize,
    maximum: usize,
}

pub fn initialize() -> io::Result<()> {
    // SAFETY: PR_SET_CHILD_SUBREAPER affects only this dedicated validator.
    if unsafe { prctl(36, 1usize, 0usize, 0usize, 0usize) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn configure(command: &mut Command, memory: usize) {
    command.process_group(0);
    // SAFETY: the post-fork closure calls only setrlimit and errno access, with
    // stack-owned data. No locks, allocation, or Rust runtime work occurs there.
    unsafe {
        command.pre_exec(move || {
            let limit = Limit {
                current: memory,
                maximum: memory,
            };
            if setrlimit(9, &limit) == -1 {
                // RLIMIT_AS, inherited by helpers.
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub fn configure_speech(command: &mut Command) {
    command.process_group(0);
}

pub struct Tree {
    group: i32,
    killed: bool,
    cleaned: bool,
}
impl Tree {
    pub fn for_speech() -> io::Result<Self> {
        Self::new(0)
    }
    pub fn new(_: usize) -> io::Result<Self> {
        Ok(Self {
            group: 0,
            killed: false,
            cleaned: false,
        })
    }
    pub fn assign(&mut self, child: &Child) -> io::Result<()> {
        self.group = i32::try_from(child.id()).map_err(io::Error::other)?;
        Ok(())
    }
    pub fn terminate(&mut self) -> io::Result<()> {
        if self.killed || self.cleaned || self.group == 0 {
            return Ok(());
        }
        // SAFETY: this private group was established at spawn. The leader has
        // not been reaped yet, so its PID cannot have been reused. Never signal
        // this numeric group again after reaping begins, including during Drop.
        if unsafe { kill(-self.group, 9) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(3) {
                return Err(error);
            } // ESRCH
        }
        self.killed = true;
        Ok(())
    }
    pub fn empty(&mut self) -> io::Result<bool> {
        if self.cleaned || self.group == 0 {
            return Ok(true);
        }
        // The caller reaps the direct child before this method. Reap only
        // descendants in this group, adopted by our dedicated subreaper.
        loop {
            let mut status = 0;
            // SAFETY: valid status pointer and our owned process group.
            let result = unsafe { waitpid(-self.group, &mut status, 1) }; // WNOHANG
            if result > 0 {
                continue;
            }
            if result == 0 {
                return Ok(false);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(4) => continue, // EINTR
                Some(10) => break,   // ECHILD
                _ => return Err(error),
            }
        }
        // Read-only check: a reused group would conservatively block cleanup,
        // never cause a signal to an unrelated process.
        if unsafe { kill(-self.group, 0) } == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(3) {
            return Err(error);
        }
        self.cleaned = true;
        Ok(true)
    }
}

pub fn parent_closed() -> ! {
    // SAFETY: only an internal worker started in a verified private group calls
    // this. Its group contains itself and its disposable native helpers.
    unsafe {
        kill(0, 9);
    }
    std::process::exit(1)
}

pub fn check_worker_group() -> io::Result<()> {
    unsafe extern "C" {
        fn getpgrp() -> i32;
        fn getpid() -> i32;
    }
    // SAFETY: these read the calling process identity without pointers.
    if unsafe { getpgrp() != getpid() } {
        return Err(io::Error::other(
            "validation worker must lead a private process group",
        ));
    }
    Ok(())
}
