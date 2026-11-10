//! Opt-in native cleanup authority, independent of local speech forwarding.
//! First integration is Linux only. Other platforms keep their existing owners
//! until their native handle/transport path has been qualified separately.
mod linux;
mod wire;

use crate::voice_validation::owned::platform;
use anyhow::{Context, Result};
use omnivox_tts::voice_library::local::{self, GuardEndpoint, Startup};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wire::{Channel, Request, TIMEOUT};

pub const MODE: &str = "--native-cleanup-guard";
pub const CLIENT: &str = "--native-guard-control";
const MAX_BOOTSTRAP: usize = 32 * 1024 * 1024;
const TICK: Duration = Duration::from_millis(5);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    worker: String,
    startup: Startup,
}

/// The actual endpoint closes before its completion lease. Every speech I/O
/// owner, including a blocked startup writer, must retain this same lease.
pub struct Io<T> {
    inner: T,
    _completion: Arc<UnixStream>,
}
impl<T: Read> Read for Io<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}
impl<T: Write> Write for Io<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub struct Peer {
    _child: Child,
    control: BufReader<UnixStream>,
    completion: Option<Arc<UnixStream>>,
    endpoint: GuardEndpoint,
    sequence: u64,
}

impl Peer {
    pub fn prepare(startup: &Startup, worker: &str) -> Result<(Self, Io<File>, Io<File>)> {
        startup.verify()?;
        let bootstrap = serde_json::to_vec(&Bootstrap {
            worker: worker.into(),
            startup: startup.clone(),
        })?;
        anyhow::ensure!(
            bootstrap.len() <= MAX_BOOTSTRAP,
            "guard startup exceeds bound"
        );
        let (mut control, inherited_control) = UnixStream::pair()?;
        let (inherited_input, input) = linux::speech_pipe()?;
        let (output, inherited_output) = linux::speech_pipe()?;
        let (completion, inherited_completion) = UnixStream::pair()?;
        let completion = Arc::new(completion);
        let stderr = linux::stderr_copy()?;
        let parent = linux::parent_identity()?;
        let child = linux::spawn(
            &startup.executable.path,
            vec![
                OwnedFd::from(inherited_control),
                OwnedFd::from(inherited_input),
                OwnedFd::from(inherited_output),
                OwnedFd::from(stderr),
                OwnedFd::from(inherited_completion),
                OwnedFd::from(parent),
            ],
        )?;
        control.set_read_timeout(Some(TIMEOUT))?;
        control.set_write_timeout(Some(TIMEOUT))?;
        control.write_all(&(bootstrap.len() as u32).to_be_bytes())?;
        control.write_all(&bootstrap)?;
        let mut control = BufReader::new(control);
        let hello = wire::read_reply(&mut control)?;
        anyhow::ensure!(
            hello["type"] == "guard_ready",
            "native guard did not prepare"
        );
        let endpoint: GuardEndpoint = serde_json::from_value(hello["endpoint"].clone())?;
        anyhow::ensure!(
            endpoint.worker == worker && endpoint.version == 1,
            "foreign native guard"
        );
        Ok((
            Self {
                _child: child,
                control,
                completion: Some(Arc::clone(&completion)),
                endpoint,
                sequence: 0,
            },
            Io {
                inner: input,
                _completion: Arc::clone(&completion),
            },
            Io {
                inner: output,
                _completion: completion,
            },
        ))
    }
    pub fn endpoint(&self) -> GuardEndpoint {
        self.endpoint.clone()
    }
    pub fn request(&mut self, command: &str) -> Result<Value> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("guard sequence exhausted")?;
        let request = Request {
            request_id: self.sequence,
            command: command.into(),
            guard: String::new(),
            worker: String::new(),
            token: String::new(),
        };
        serde_json::to_writer(self.control.get_mut(), &request)?;
        self.control.get_mut().write_all(b"\n")?;
        let reply = wire::read_reply(&mut self.control)?;
        anyhow::ensure!(
            reply["request_id"] == self.sequence
                && reply["guard"] == self.endpoint.guard
                && reply["worker"] == self.endpoint.worker,
            "foreign native guard reply"
        );
        if let Some(error) = reply["error"].as_str() {
            anyhow::bail!("{error}");
        }
        Ok(reply)
    }
    pub fn start(&mut self) -> Result<()> {
        let reply = self.request("start")?;
        anyhow::ensure!(
            reply["started"] == true && reply["tree_retired"] == false,
            "guard did not establish native ownership"
        );
        Ok(())
    }
    pub fn retire_tree(&mut self) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        let mut status = self.request("retire")?;
        while status["tree_retired"] != true {
            anyhow::ensure!(
                Instant::now() < deadline,
                "guard tree retirement unconfirmed"
            );
            std::thread::sleep(TICK);
            status = self.request("status")?;
        }
        Ok(())
    }
    pub fn finish_io(&mut self) -> Result<()> {
        self.completion.take();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let status = self.request("status")?;
            if status["tree_retired"] == true && status["io_retired"] == true {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "guard I/O retirement unconfirmed"
            );
            std::thread::sleep(TICK);
        }
    }
}

struct Native {
    child: Child,
    tree: platform::Tree,
    assigned: bool,
    retired: bool,
}
impl Native {
    fn poll(&mut self) -> Result<()> {
        if self.retired {
            return Ok(());
        }
        if self.assigned {
            self.tree.terminate()?;
        } else {
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
        let deadline = Instant::now() + TIMEOUT;
        while !self.retired && Instant::now() < deadline {
            let _ = self.poll();
            std::thread::sleep(TICK);
        }
    }
}

struct Driver {
    // Drop native ownership before any other teardown; no file/history writes.
    native: Option<Native>,
    pipes: Option<[File; 3]>,
    startup: Option<Startup>,
    parent: File,
    completion: UnixStream,
    endpoint: GuardEndpoint,
    attempted: bool,
    closing: bool,
    startup_error: Option<String>,
    io_retired: bool,
    client_attached: bool,
}
impl Driver {
    fn tree_retired(&self) -> bool {
        self.closing
            && self.pipes.is_none()
            && self.native.as_ref().is_none_or(|native| native.retired)
    }
    fn retire(&mut self) {
        self.closing = true;
        self.startup.take();
        self.pipes.take();
    }
    fn start(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.client_attached && !self.closing,
            "guard client must attach before native start"
        );
        if self.attempted {
            anyhow::ensure!(self.native.is_some(), "native attempt already failed");
            return Ok(());
        }
        self.attempted = true;
        let startup = self.startup.take().context("missing guard startup")?;
        startup.verify()?;
        anyhow::ensure!(
            !linux::ended(&self.parent)?,
            "forwarding owner ended before native start"
        );
        let [input, output, error] = self.pipes.take().context("guard pipes already consumed")?;
        let tree = platform::Tree::for_speech()?;
        let mut command = Command::new(&startup.executable.path);
        command
            .args(&startup.arguments)
            .current_dir(&startup.working_directory);
        startup.environment.apply(&mut command);
        command
            .env_remove("OMNIVOX_REMOTE_WORKER")
            .env("OMNIVOX_OWNED_WORKER", "1")
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(error));
        platform::configure_speech(&mut command);
        let child = command.spawn()?;
        drop(command);
        self.native = Some(Native {
            child,
            tree,
            assigned: false,
            retired: false,
        });
        let native = self.native.as_mut().unwrap();
        native.tree.assign(&native.child)?;
        native.assigned = true;
        // The actual owner still has to write the validated owned START frame.
        Ok(())
    }
    fn poll(&mut self) -> Result<()> {
        if linux::ended(&self.parent)? {
            self.retire();
        }
        if !self.io_retired {
            match self.completion.read(&mut [0]) {
                Ok(0) => self.io_retired = true,
                Ok(_) => anyhow::bail!("invalid completion lease"),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if self.closing {
            if let Some(native) = &mut self.native {
                native.poll()?;
            }
        }
        Ok(())
    }
    fn reply(&self, id: u64, error: Option<String>) -> Value {
        json!({"type":"guard_status","request_id":id,"guard":self.endpoint.guard,"worker":self.endpoint.worker,
            "attempted":self.attempted,"started":self.native.is_some(),"tree_retired":self.tree_retired(),
            "io_retired":self.io_retired,"error":error})
    }
    fn command(&mut self, request: &Request, parent: bool) -> Value {
        let result: Result<()> = (|| {
            match request.command.as_str() {
                "start" if parent => {
                    if let Err(error) = self.start() {
                        if self.attempted {
                            self.startup_error = Some(error.to_string());
                            self.retire();
                        }
                        return Err(error);
                    }
                    if let Some(error) = &self.startup_error {
                        anyhow::bail!("{error}");
                    }
                }
                "retire" => self.retire(),
                "retire-owner" if !parent => {
                    anyhow::ensure!(
                        self.tree_retired(),
                        "native tree must retire before its forwarding owner"
                    );
                    linux::terminate_parent(&self.parent)?;
                }
                "status" => (),
                _ => anyhow::bail!("unsupported guard operation"),
            }
            self.poll()
        })();
        self.reply(request.request_id, result.err().map(|e| e.to_string()))
    }
}

fn same_user(stream: &UnixStream) -> io::Result<bool> {
    let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = size_of_val(&credential) as libc::socklen_t;
    // SAFETY: actual connected socket and exact native credential storage.
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
    Ok(credential.uid == unsafe { libc::geteuid() })
}

fn authorized(request: &Request, endpoint: &GuardEndpoint) -> bool {
    request.command == "attach"
        && request.guard == endpoint.guard
        && request.worker == endpoint.worker
        && request.token.len() == endpoint.token.len()
        && request
            .token
            .bytes()
            .zip(endpoint.token.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub fn run(arguments: &[String]) -> Result<()> {
    // SAFETY: dispatched at executable entry before any files/threads/native work.
    let (mut control, input, output, error, completion, parent) =
        unsafe { linux::receive(arguments)? };
    control.set_read_timeout(Some(TIMEOUT))?;
    let mut length = [0; 4];
    control.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    anyhow::ensure!(
        length > 0 && length <= MAX_BOOTSTRAP,
        "invalid guard bootstrap size"
    );
    let mut bytes = vec![0; length];
    control.read_exact(&mut bytes)?;
    let bootstrap: Bootstrap = serde_json::from_slice(&bytes)?;
    platform::initialize()?;
    let guard = local::new_uuid()?;
    let address = format!("omnivox-guard-{guard}");
    let listener = UnixListener::bind_addr(&SocketAddr::from_abstract_name(address.as_bytes())?)?;
    listener.set_nonblocking(true)?;
    control.set_nonblocking(true)?;
    control.set_read_timeout(None)?;
    completion.set_nonblocking(true)?;
    let endpoint = GuardEndpoint {
        version: 1,
        guard,
        worker: bootstrap.worker,
        transport: "linux-abstract".into(),
        address,
        token: format!(
            "{}{}",
            local::new_uuid()?.replace('-', ""),
            local::new_uuid()?.replace('-', "")
        ),
    };
    let mut parent_channel = Channel::new(control);
    parent_channel.reply(&json!({"type":"guard_ready","endpoint":endpoint}))?;
    let mut driver = Driver {
        native: None,
        pipes: Some([input, output, error]),
        startup: Some(bootstrap.startup),
        parent,
        completion,
        endpoint,
        attempted: false,
        closing: false,
        startup_error: None,
        io_retired: false,
        client_attached: false,
    };
    let mut client: Option<(Channel<UnixStream>, bool, Instant)> = None;
    loop {
        driver.poll()?;
        match parent_channel.poll() {
            Ok(Some(request)) => parent_channel.reply(&driver.command(&request, true))?,
            Ok(None) => (),
            Err(_) => parent_channel.closed = true,
        }
        if parent_channel.closed {
            driver.retire();
        }
        // One connection total, including unauthenticated handshakes.
        if client.is_none() {
            match listener.accept() {
                Ok((stream, _)) if same_user(&stream)? => {
                    stream.set_nonblocking(true)?;
                    client = Some((Channel::new(stream), false, Instant::now()));
                }
                Ok(_) => (),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        if let Some((channel, authenticated, since)) = &mut client {
            match channel.poll() {
                Ok(Some(request)) if !*authenticated => {
                    if authorized(&request, &driver.endpoint) {
                        *authenticated = true;
                        driver.client_attached = true;
                        channel.reply(&driver.reply(request.request_id, None))?;
                    } else {
                        channel.closed = true;
                    }
                }
                Ok(Some(request)) => {
                    if request.guard != driver.endpoint.guard
                        || request.worker != driver.endpoint.worker
                    {
                        channel.closed = true;
                    } else {
                        channel.reply(&driver.command(&request, false))?;
                    }
                }
                Ok(None) => (),
                Err(_) => channel.closed = true,
            }
            if !*authenticated && since.elapsed() > TIMEOUT {
                channel.closed = true;
            }
            if channel.closed {
                client = None;
                driver.client_attached = false;
            }
        }
        // A lost client link alone preserves healthy speech. Once both owners
        // are gone, retire and exit; no missing reply becomes cleanup evidence.
        if parent_channel.closed && client.is_none() && driver.tree_retired() && driver.io_retired {
            return Ok(());
        }
        std::thread::sleep(TICK);
    }
}

/// A private stdio bridge for clients without native abstract-socket support.
/// It carries only bounded lifecycle frames, never speech or native handles.
pub fn bridge() -> Result<()> {
    let mut input = BufReader::new(io::stdin());
    let endpoint: GuardEndpoint = serde_json::from_value(wire::read_reply(&mut input)?)?;
    anyhow::ensure!(
        endpoint.version == 1
            && endpoint.transport == "linux-abstract"
            && endpoint.address == format!("omnivox-guard-{}", endpoint.guard),
        "unsupported guard endpoint"
    );
    let mut stream = UnixStream::connect_addr(&SocketAddr::from_abstract_name(
        endpoint.address.as_bytes(),
    )?)?;
    let mut sender = stream.try_clone()?;
    // Input is bounded before forwarding, and shutdown wakes a blocked receive
    // if the actual client closes stdin. This bridge owns no native worker.
    std::thread::Builder::new()
        .name("guard-control-input".into())
        .spawn(move || {
            let result: Result<()> = (|| loop {
                let mut line = Vec::new();
                (&mut input)
                    .take((wire::MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut line)?;
                if line.is_empty() {
                    return Ok(());
                }
                anyhow::ensure!(
                    line.len() <= wire::MAX_FRAME && line.last() == Some(&b'\n'),
                    "guard bridge input exceeds bound"
                );
                sender.write_all(&line)?;
            })();
            let _ = sender.shutdown(Shutdown::Both);
            if let Err(error) = result {
                eprintln!("Guard control input ended: {error}");
            }
        })?;
    let mut output = io::stdout().lock();
    let mut bytes = [0; wire::MAX_FRAME];
    loop {
        let n = stream.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        output.write_all(&bytes[..n])?;
        output.flush()?;
    }
    Ok(())
}
