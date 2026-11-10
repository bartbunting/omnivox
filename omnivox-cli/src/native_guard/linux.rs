//! Private inherited descriptors. No saved PID grants signalling authority.
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

pub fn parent_identity() -> io::Result<File> {
    // SAFETY: open a handle to this very process, before creating its guard.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful syscall returned a fresh owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd as i32) })
}

pub fn ended(parent: &File) -> io::Result<bool> {
    let mut fd = libc::pollfd {
        fd: parent.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one live descriptor and valid pollfd storage; no blocking wait.
    if unsafe { libc::poll(&mut fd, 1, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if fd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(io::Error::other("invalid parent lifetime handle"));
    }
    Ok(fd.revents & libc::POLLIN != 0)
}

pub fn terminate_parent(parent: &File) -> io::Result<()> {
    if ended(parent)? {
        return Ok(());
    }
    // SAFETY: retained actual parent pidfd, never an opened numerical PID.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            parent.as_raw_fd(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if rc < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn stderr_copy() -> io::Result<File> {
    // SAFETY: duplicate standard error into a fresh close-on-exec descriptor.
    let fd = unsafe { libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fresh descriptor with sole ownership.
    Ok(unsafe { File::from_raw_fd(fd) })
}

pub fn speech_pipe() -> io::Result<(File, File)> {
    let mut descriptors = [-1; 2];
    // SAFETY: storage for both newly allocated pipe descriptors. Keep the
    // ordinary worker's anonymous-pipe transport, with no inherited extras.
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe2 returned distinct descriptors, each with one Rust owner.
    Ok(unsafe {
        (
            File::from_raw_fd(descriptors[0]),
            File::from_raw_fd(descriptors[1]),
        )
    })
}

pub fn spawn(executable: &str, files: Vec<OwnedFd>) -> io::Result<Child> {
    let mut command = Command::new(executable);
    command
        .arg(super::MODE)
        .args(files.iter().map(|fd| fd.as_raw_fd().to_string()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .process_group(0);
    // SAFETY: only async-signal-safe fcntl on owned, distinct descriptors after
    // fork. Parent copies remain CLOEXEC. Command is dropped just after spawn.
    unsafe {
        command.pre_exec(move || {
            for fd in &files {
                if libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    command.spawn()
}

/// # Safety
/// Called once at the private entry before opening files or starting threads.
/// The six inherited values have no other Rust owners in this process.
pub unsafe fn receive(
    arguments: &[String],
) -> io::Result<(UnixStream, File, File, File, UnixStream, File)> {
    let fds: Vec<i32> = arguments
        .iter()
        .map(|s| s.parse().map_err(io::Error::other))
        .collect::<io::Result<_>>()?;
    if fds.len() != 6
        || fds
            .iter()
            .enumerate()
            .any(|(i, fd)| *fd < 3 || fds[..i].contains(fd))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for fd in &fds {
        // SAFETY: fcntl validates the inherited value before ownership is taken.
        if unsafe { libc::fcntl(*fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = size_of_val(&credential) as libc::socklen_t;
    // SAFETY: exact native output storage. The private control pair was created
    // by our actual parent, not accepted from a pathname or remote connection.
    if unsafe {
        libc::getsockopt(
            fds[0],
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credential).cast(),
            &mut len,
        )
    } < 0
        || credential.pid != unsafe { libc::getppid() }
        || credential.uid != unsafe { libc::geteuid() }
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    // SAFETY: caller transfers the six validated distinct inherited descriptors.
    Ok(unsafe {
        (
            UnixStream::from_raw_fd(fds[0]),
            File::from_raw_fd(fds[1]),
            File::from_raw_fd(fds[2]),
            File::from_raw_fd(fds[3]),
            UnixStream::from_raw_fd(fds[4]),
            File::from_raw_fd(fds[5]),
        )
    })
}
