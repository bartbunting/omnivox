//! Bounded subprocess capture. The pinned frontend and runtime spawn no children.
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(5);
const RETIRE: Duration = Duration::from_secs(2);

struct OwnedChild(Child);
impl std::ops::Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = self.0.kill();
        let deadline = Instant::now() + RETIRE;
        while !matches!(self.0.try_wait(), Ok(Some(_))) {
            if Instant::now() >= deadline {
                std::process::exit(70);
            }
            thread::sleep(POLL);
        }
    }
}

fn read_bounded(mut input: impl Read, maximum: usize) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    input
        .by_ref()
        .take(maximum as u64 + 1)
        .read_to_end(&mut output)?;
    if output.len() > maximum {
        return Err(io::Error::other("native output exceeds prototype limit"));
    }
    Ok(output)
}

/// Always reap the child and finish all pipe readers before allowing reuse.
/// If retirement cannot be proved, exit the owning helper; its host recovers it.
pub fn capture(
    command: &mut Command,
    input: Vec<u8>,
    maximum: usize,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> io::Result<Vec<u8>> {
    if cancelled() {
        return Err(io::Error::other("MBROLA request cancelled before spawn"));
    }
    configure_child(command)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Native inputs never receive parent eSpeak/MBROLA path overrides or locale.
    command
        .env_remove("ESPEAK_DATA_PATH")
        .env_remove("ESPEAK_NG_DATA")
        .env("LC_ALL", "C");
    let mut child = OwnedChild(command.spawn()?);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let out_tx = tx.clone();
    let err_tx = tx.clone();
    // Exactly three bounded messages; threads cannot outlive successful retirement.
    let writer = thread::spawn(move || {
        let _ = tx.send((0, stdin.write_all(&input).map(|()| Vec::new())));
    });
    let reader = thread::spawn(move || {
        let _ = out_tx.send((1, read_bounded(stdout, maximum)));
    });
    let errors = thread::spawn(move || {
        let _ = err_tx.send((2, read_bounded(stderr, 16 * 1024)));
    });
    let deadline = Instant::now() + timeout;
    let mut outputs: [Option<io::Result<Vec<u8>>>; 3] = [None, None, None];
    let mut failure = None;
    let status = loop {
        for (kind, result) in rx.try_iter() {
            if let Err(error) = &result {
                failure = Some(io::Error::other(error.to_string()));
            }
            outputs[kind] = Some(result);
        }
        if cancelled() || Instant::now() >= deadline {
            failure = Some(io::Error::other("MBROLA request cancelled or timed out"));
        }
        if failure.is_some() {
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) if outputs.iter().all(Option::is_some) => break Some(status),
            Ok(_) => thread::sleep(POLL),
            Err(error) => {
                failure = Some(error);
                break None;
            }
        }
    };
    if status.is_none() {
        let _ = child.kill();
    }
    let retire_deadline = Instant::now() + RETIRE;
    loop {
        for (kind, result) in rx.try_iter() {
            outputs[kind] = Some(result);
        }
        if matches!(child.try_wait(), Ok(Some(_))) && outputs.iter().all(Option::is_some) {
            break;
        }
        if Instant::now() >= retire_deadline {
            eprintln!("MBROLA native child retirement unconfirmed; retiring helper");
            std::process::exit(70);
        }
        thread::sleep(POLL);
    }
    for thread in [writer, reader, errors] {
        thread
            .join()
            .map_err(|_| io::Error::other("native pipe worker panicked"))?;
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    let stderr = outputs[2].take().unwrap()?;
    if !status.unwrap().success() {
        return Err(io::Error::other(format!(
            "native process failed ({}): {}",
            status.unwrap(),
            String::from_utf8_lossy(&stderr)
        )));
    }
    outputs[0].take().unwrap()?;
    outputs[1].take().unwrap()
}

#[cfg(target_os = "linux")]
fn configure_child(command: &mut Command) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
        fn getppid() -> i32;
        fn getpid() -> i32;
    }
    // SAFETY: getpid has no arguments or memory access.
    let parent = unsafe { getpid() };
    // SAFETY: only async-signal-safe Linux syscalls run after fork. No allocation.
    unsafe {
        command.pre_exec(move || {
            if prctl(1, 9usize, 0usize, 0usize, 0usize) != 0 {
                // PR_SET_PDEATHSIG, SIGKILL
                return Err(io::Error::last_os_error());
            }
            if getppid() != parent {
                return Err(io::Error::from_raw_os_error(3));
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(windows)]
fn configure_child(command: &mut Command) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; inherit our private job.
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn configure_child(_: &mut Command) -> io::Result<()> {
    Err(io::Error::other(
        "MBROLA prototype supports Linux and Windows only",
    ))
}

#[cfg(not(windows))]
pub struct Ownership;

#[cfg(not(windows))]
pub fn own_helper() -> io::Result<Ownership> {
    Ok(Ownership)
}

#[cfg(windows)]
pub use windows::own_helper;

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Basic {
        process_time: i64,
        job_time: i64,
        flags: u32,
        minimum: usize,
        maximum: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Limits {
        basic: Basic,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, data: *const c_void, size: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
    }
    pub struct Job(Handle);
    pub fn own_helper() -> io::Result<Job> {
        // SAFETY: create an unnamed, non-inheritable owning handle.
        let job = Job(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
        if job.0.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits = Limits::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                                     // SAFETY: repr(C) layout is JOBOBJECT_EXTENDED_LIMIT_INFORMATION, class 9.
        if unsafe {
            SetInformationJobObject(
                job.0,
                9,
                &limits as *const _ as _,
                std::mem::size_of::<Limits>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: own this helper before any descendants or synthesis threads exist.
        if unsafe { AssignProcessToJobObject(job.0, GetCurrentProcess()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: unique owning handle; closing retires the private process tree.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn bounds_output_and_retires_blocked_input_before_reuse() {
        let mut flood = Command::new("/bin/sh");
        flood.args(["-c", "while :; do printf 0123456789; done"]);
        assert!(capture(&mut flood, vec![], 100, Duration::from_secs(1), || false).is_err());
        let mut blocked = Command::new("/bin/sleep");
        blocked.arg("10");
        let start = Instant::now();
        assert!(capture(
            &mut blocked,
            vec![b'x'; 1024 * 1024],
            100,
            Duration::from_millis(50),
            || false
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut next = Command::new("/bin/cat");
        assert_eq!(
            capture(
                &mut next,
                b"replacement".to_vec(),
                100,
                Duration::from_secs(1),
                || false
            )
            .unwrap(),
            b"replacement"
        );
    }
}
