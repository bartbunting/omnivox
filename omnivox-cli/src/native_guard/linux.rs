//! Private inherited descriptors. No saved PID grants signalling authority.
use omnivox_tts::voice_library::local::GuardEndpoint;
use std::fs::File;
use std::io;
use std::io::{BufRead, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::UnixStream;
use std::os::unix::net::{SocketAddr, UnixListener};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

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

pub type Parent = File;
pub type Completion = UnixStream;
pub type Lease = UnixStream;
pub type Blocking = UnixStream;
pub type Control = UnixStream;
pub type GuardChild = Child;
pub const TRANSPORT: &str = "linux-abstract";
pub struct Spawned {
    pub child: GuardChild,
    pub control: Blocking,
    pub input: File,
    pub output: File,
    pub completion: Arc<Lease>,
}
pub fn launch(executable: &str) -> io::Result<Spawned> {
    let (control, inherited_control) = UnixStream::pair()?;
    let (inherited_input, input) = speech_pipe()?;
    let (output, inherited_output) = speech_pipe()?;
    let (completion, inherited_completion) = UnixStream::pair()?;
    let child = spawn(
        executable,
        vec![
            OwnedFd::from(inherited_control),
            OwnedFd::from(inherited_input),
            OwnedFd::from(inherited_output),
            OwnedFd::from(stderr_copy()?),
            OwnedFd::from(inherited_completion),
            OwnedFd::from(parent_identity()?),
        ],
    )?;
    Ok(Spawned {
        child,
        control,
        input,
        output,
        completion: Arc::new(completion),
    })
}
pub fn timeouts(stream: &mut Blocking, timeout: Option<Duration>) -> io::Result<()> {
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)
}
pub fn into_channel(stream: Blocking) -> io::Result<Control> {
    stream.set_nonblocking(true)?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok(stream)
}
pub fn completed(completion: &mut Completion, _parent: &Parent) -> io::Result<bool> {
    completion.set_nonblocking(true)?;
    match completion.read(&mut [0]) {
        Ok(0) => Ok(true),
        Ok(_) => Err(io::Error::other("invalid completion lease")),
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
pub fn close(_stream: &mut Control) -> io::Result<bool> {
    Ok(true)
}
pub struct Listener {
    inner: UnixListener,
    address: String,
}
impl Listener {
    pub fn bind(id: &str) -> io::Result<Self> {
        let address = format!("omnivox-guard-{id}");
        let inner = UnixListener::bind_addr(&SocketAddr::from_abstract_name(address.as_bytes())?)?;
        inner.set_nonblocking(true)?;
        Ok(Self { inner, address })
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn accept(&mut self) -> io::Result<Option<Control>> {
        match self.inner.accept() {
            Ok((stream, _)) => {
                let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
                let mut size = size_of_val(&credential) as libc::socklen_t;
                // SAFETY: exact native storage and actual connected socket.
                if unsafe {
                    libc::getsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_PEERCRED,
                        (&raw mut credential).cast(),
                        &mut size,
                    )
                } < 0
                {
                    return Err(io::Error::last_os_error());
                }
                if credential.uid != unsafe { libc::geteuid() } {
                    return Ok(None);
                }
                stream.set_nonblocking(true)?;
                Ok(Some(stream))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}
impl AsRawFd for Listener {
    fn as_raw_fd(&self) -> i32 {
        self.inner.as_raw_fd()
    }
}
pub fn bridge(
    endpoint: GuardEndpoint,
    mut input: std::io::BufReader<std::io::Stdin>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        endpoint.address == format!("omnivox-guard-{}", endpoint.guard),
        "invalid guard address"
    );
    let mut stream = UnixStream::connect_addr(&SocketAddr::from_abstract_name(
        endpoint.address.as_bytes(),
    )?)?;
    let mut sender = stream.try_clone()?;
    std::thread::Builder::new()
        .name("guard-control-input".into())
        .spawn(move || {
            let result: anyhow::Result<()> = (|| loop {
                let mut line = Vec::new();
                (&mut input)
                    .take((super::wire::MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut line)?;
                if line.is_empty() {
                    return Ok(());
                }
                anyhow::ensure!(
                    line.len() <= super::wire::MAX_FRAME && line.last() == Some(&b'\n'),
                    "guard bridge input exceeds bound"
                );
                sender.write_all(&line)?;
            })();
            let _ = sender.shutdown(std::net::Shutdown::Both);
            if let Err(error) = result {
                eprintln!("Guard control input ended: {error}");
            }
        })?;
    let mut output = std::io::stdout().lock();
    let mut bytes = [0; super::wire::MAX_FRAME];
    loop {
        let n = stream.read(&mut bytes)?;
        if n == 0 {
            return Ok(());
        }
        output.write_all(&bytes[..n])?;
        output.flush()?;
    }
}
