//! Linux-only extraction experiment, not an installed Omnivox entry point.
//! Speech descriptors bypass this process. Lifecycle polling never writes files
//! or speech, and a blocked status consumer cannot stop native retirement.
#![allow(dead_code)]

#[path = "platform-linux.rs"]
mod platform;

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn fcntl(fd: i32, command: i32, ...) -> i32;
}

fn prepare(file: &File, nonblocking: bool) -> io::Result<()> {
    let fd = file.as_raw_fd();
    // SAFETY: fd is owned here. Linux F_SETFD/F_GETFL/F_SETFL have no pointers.
    unsafe {
        if fcntl(fd, 2, 1) < 0 {
            return Err(io::Error::last_os_error());
        }
        if nonblocking {
            let flags = fcntl(fd, 3);
            if flags < 0 || fcntl(fd, 4, flags | 0x800) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn closed(file: &mut File) -> io::Result<bool> {
    match file.read(&mut [0]) {
        Ok(0) => Ok(true),
        Ok(_) => Err(io::Error::other("unexpected lifetime data")),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

struct Native {
    child: Child,
    tree: platform::Tree,
    assigned: bool,
    retired: bool,
}

impl Native {
    fn poll_retirement(&mut self) -> io::Result<()> {
        if self.retired {
            return Ok(());
        }
        if self.assigned {
            // The leader is deliberately unreaped until this first signal.
            self.tree.terminate()?;
        } else {
            // Native START is still closed if assignment failed.
            self.child.kill()?;
        }
        if self.child.try_wait()?.is_some() && self.tree.empty()? {
            self.retired = true;
        }
        Ok(())
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.retired && Instant::now() < deadline {
            let _ = self.poll_retirement();
            std::thread::sleep(Duration::from_millis(5));
        }
        // Failure emits no cleanup proof. No numerical PID is reopened/signalled.
    }
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 6 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let fds: Vec<i32> = args[..5]
        .iter()
        .map(|s| s.parse().map_err(io::Error::other))
        .collect::<io::Result<_>>()?;
    if fds.iter().any(|fd| *fd < 3) || fds.iter().enumerate().any(|(i, fd)| fds[..i].contains(fd)) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // SAFETY: this fixture is launched with exactly these five inherited files;
    // each descriptor is transferred once before threads or native children exist.
    let mut files: Vec<File> = fds
        .iter()
        .map(|fd| unsafe { File::from_raw_fd(*fd) })
        .collect();
    for (i, file) in files.iter().enumerate() {
        prepare(file, i >= 3)?;
    }
    let mut completion = files.pop().unwrap();
    let mut lifetime = files.pop().unwrap();
    let stderr = files.pop().unwrap();
    let stdout = files.pop().unwrap();
    let stdin = files.pop().unwrap();
    platform::initialize()?;
    let tree = platform::Tree::for_speech()?;
    let mut command = Command::new(&args[5]);
    command
        .args(&args[6..])
        .env("OMNIVOX_OWNED_WORKER", "1")
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    platform::configure_speech(&mut command);
    let child = command.spawn()?;
    drop(command); // No child pipe endpoints may keep I/O alive in this guard.
    let mut native = Native {
        child,
        tree,
        assigned: false,
        retired: false,
    };
    native.tree.assign(&native.child)?;
    native.assigned = true;
    // SAFETY: this dedicated entry has created no Rust stdin/stdout owners.
    let (mut control, mut replies) = unsafe { (File::from_raw_fd(0), File::from_raw_fd(1)) };
    prepare(&control, true)?;
    prepare(&replies, true)?;
    let worker = native.child.id();
    let mut pending = format!("READY {worker}\n").into_bytes();
    let mut offset = 0;
    let mut retiring = false;
    let mut io_retired = false;
    let mut client_lost = false;
    let mut sequence = 0u64;
    let mut output_stalls = 0u64;
    loop {
        retiring |= closed(&mut lifetime)?;
        io_retired |= closed(&mut completion)?;
        let mut input = [0; 128];
        let mut query = false;
        match control.read(&mut input) {
            Ok(0) => {
                client_lost = true;
                retiring = true;
            }
            Ok(n) => {
                for byte in &input[..n] {
                    match byte {
                        b'R' => {
                            retiring = true;
                            query = true;
                        }
                        b'Q' => query = true,
                        _ => {
                            retiring = true;
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                ()
            }
            Err(_) => {
                client_lost = true;
                retiring = true;
            }
        }
        if retiring {
            native.poll_retirement()?;
        }
        // At most one bounded reply. A partial frame is never overwritten.
        if query && pending.is_empty() {
            sequence += 1;
            pending = format!(
                "STATUS {sequence} {worker} {} {} {output_stalls}\n",
                native.retired, io_retired
            )
            .into_bytes();
        }
        if !pending.is_empty() {
            match replies.write(&pending[offset..]) {
                Ok(0) => {
                    client_lost = true;
                    retiring = true;
                }
                Ok(n) => {
                    offset += n;
                    if offset == pending.len() {
                        pending.clear();
                        offset = 0;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    output_stalls = output_stalls.saturating_add(1);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    client_lost = true;
                    retiring = true;
                }
            }
        }
        if client_lost && native.retired && io_retired {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
