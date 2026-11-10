//! Opt-in native cleanup authority, independent of local speech forwarding.
//! Native lifetime and local control use platform-specific handles. Speech
//! retains the ordinary anonymous-pipe path and never passes through the guard.
#[cfg(windows)]
#[path = "native_guard/windows/helper_client.rs"]
mod helper_client;
#[cfg(windows)]
#[path = "native_guard/windows/helper_wire.rs"]
mod helper_wire;
#[cfg(target_os = "linux")]
#[path = "native_guard/linux.rs"]
mod os;
#[cfg(windows)]
#[path = "native_guard/windows.rs"]
mod os;
#[cfg(target_os = "linux")]
mod readiness;
#[cfg(windows)]
#[path = "native_guard/windows/readiness.rs"]
mod readiness;
#[cfg(windows)]
#[path = "native_guard/windows/owned.rs"]
mod windows_owned;
#[cfg(windows)]
#[path = "native_guard/windows/plan.rs"]
mod windows_plan;
mod wire;
#[cfg(windows)]
use windows_owned::Native;

use crate::voice_validation::owned::platform;
use anyhow::{Context, Result};
use omnivox_tts::voice_library::local::{self, GuardEndpoint, Startup};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
#[cfg(target_os = "linux")]
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wire::{Channel, Request, TIMEOUT};

pub const MODE: &str = "--native-cleanup-guard";
pub const CLIENT: &str = "--native-guard-control";
const MAX_BOOTSTRAP: usize = 32 * 1024 * 1024;
const TICK: Duration = Duration::from_millis(5);

#[cfg(windows)]
pub fn worker_arguments(mut args: Vec<String>) -> Result<Vec<String>> {
    if args
        .first()
        .is_some_and(|arg| arg == os::native::creation::WORKER_MODE)
    {
        let handle = args
            .get(1)
            .context("missing private helper creation channel")?;
        helper_client::install(handle)?;
        args.drain(..2);
    }
    Ok(args)
}

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
    _completion: Arc<os::Lease>,
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
    _child: os::GuardChild,
    control: BufReader<os::Blocking>,
    completion: Option<Arc<os::Lease>>,
    endpoint: GuardEndpoint,
    sequence: u64,
}

impl Peer {
    pub fn prepare(startup: &Startup, worker: &str) -> Result<(Self, Io<File>, Io<File>)> {
        #[cfg(windows)]
        windows_plan::Plan::validate(startup)?;
        startup.verify()?;
        let bootstrap = serde_json::to_vec(&Bootstrap {
            worker: worker.into(),
            startup: startup.clone(),
        })?;
        anyhow::ensure!(
            bootstrap.len() <= MAX_BOOTSTRAP,
            "guard startup exceeds bound"
        );
        let os::Spawned {
            child,
            mut control,
            input,
            output,
            completion,
        } = os::launch(&startup.executable.path)?;
        os::timeouts(&mut control, Some(TIMEOUT))?;
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
        let deadline = Instant::now() + TIMEOUT;
        let mut reply = self.request("start")?;
        while reply["started"] != true && reply["tree_retired"] != true {
            anyhow::ensure!(
                reply["native_error"].is_null(),
                "native start failed: {}",
                reply["native_error"]
            );
            anyhow::ensure!(Instant::now() < deadline, "guard native start unconfirmed");
            std::thread::sleep(TICK);
            reply = self.request("status")?;
        }
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

#[cfg(target_os = "linux")]
struct Native {
    child: Child,
    tree: platform::Tree,
    assigned: bool,
    retired: bool,
}
#[cfg(target_os = "linux")]
impl Native {
    fn poll(&mut self) -> Result<()> {
        if self.retired {
            return Ok(());
        }
        if self.assigned {
            self.tree.terminate()?;
        } else if let Err(error) = self.child.kill() {
            // An already-exited unassigned child is still confirmed by its
            // retained Child handle. A failed kill alone never releases it.
            if self.child.try_wait()?.is_none() {
                return Err(error.into());
            }
        }
        if self.child.try_wait()?.is_some() && self.tree.empty()? {
            self.retired = true;
        }
        Ok(())
    }
}
#[cfg(target_os = "linux")]
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
    #[cfg(windows)]
    plan: Option<windows_plan::Plan>,
    parent: os::Parent,
    completion: os::Completion,
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
        #[cfg(windows)]
        self.plan.take();
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
        anyhow::ensure!(
            !os::ended(&self.parent)?,
            "forwarding owner ended before native start"
        );
        let [input, output, error] = self.pipes.take().context("guard pipes already consumed")?;
        #[cfg(windows)]
        {
            self.native = Some(Native::start(
                startup,
                [input, output, error],
                self.plan.take().context("missing native creation plan")?,
            )?);
        }
        #[cfg(target_os = "linux")]
        {
            startup.verify()?;
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
        }
        // The actual owner still has to write the validated owned START frame.
        Ok(())
    }
    fn poll(&mut self) -> Result<()> {
        if os::ended(&self.parent)? {
            self.retire();
        }
        if !self.io_retired {
            self.io_retired = os::completed(&mut self.completion, &self.parent)?;
        }
        #[cfg(windows)]
        if let Some(native) = &mut self.native {
            native.poll(self.closing)?;
            if native.closing {
                self.retire();
            }
        }
        #[cfg(target_os = "linux")]
        if self.closing {
            if let Some(native) = &mut self.native {
                native.poll()?;
            }
        }
        Ok(())
    }
    fn reply(&self, id: u64, error: Option<String>) -> Value {
        #[cfg(windows)]
        let started = self.native.as_ref().is_some_and(|native| native.started);
        #[cfg(target_os = "linux")]
        let started = self.native.is_some();
        #[cfg(windows)]
        let native_error = self
            .native
            .as_ref()
            .and_then(|native| native.error.as_deref());
        #[cfg(target_os = "linux")]
        let native_error: Option<&str> = None;
        json!({"type":"guard_status","request_id":id,"guard":self.endpoint.guard,"worker":self.endpoint.worker,
            "attempted":self.attempted,"started":started,"tree_retired":self.tree_retired(),
            "io_retired":self.io_retired,"error":error,"native_error":native_error})
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
                    os::terminate_parent(&self.parent)?;
                }
                "status" => (),
                _ => anyhow::bail!("unsupported guard operation"),
            }
            self.poll()
        })();
        self.reply(request.request_id, result.err().map(|e| e.to_string()))
    }
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
        unsafe { os::receive(arguments)? };
    os::timeouts(&mut control, Some(TIMEOUT))?;
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
    #[cfg(windows)]
    let plan = windows_plan::Plan::prepare(&bootstrap.startup)?;
    platform::initialize()?;
    let guard = local::new_uuid()?;
    let listener = os::Listener::bind(&guard)?;
    let address = listener.address().to_owned();
    let control = os::into_channel(control)?;
    let endpoint = GuardEndpoint {
        version: 1,
        guard,
        worker: bootstrap.worker,
        transport: os::TRANSPORT.into(),
        address,
        token: format!(
            "{}{}",
            local::new_uuid()?.replace('-', ""),
            local::new_uuid()?.replace('-', "")
        ),
    };
    let mut parent_channel = Channel::new(control);
    parent_channel.reply(&json!({"type":"guard_ready","endpoint":endpoint}))?;
    let driver = Driver {
        native: None,
        pipes: Some([input, output, error]),
        startup: Some(bootstrap.startup),
        #[cfg(windows)]
        plan: Some(plan),
        parent,
        completion,
        endpoint,
        attempted: false,
        closing: false,
        startup_error: None,
        io_retired: false,
        client_attached: false,
    };
    let mut running = Running {
        driver,
        parent_channel,
        client: None,
        listener,
    };
    running.serve()
}

// Drop native ownership first even on an I/O error. Windows control cancellation
// retains its native allocations until completion and may wait in Drop.
struct Running {
    driver: Driver,
    parent_channel: Channel<os::Control>,
    client: Option<(Channel<os::Control>, bool, Instant)>,
    listener: os::Listener,
}
impl Running {
    fn serve(&mut self) -> Result<()> {
        let Self {
            driver,
            parent_channel,
            client,
            listener,
        } = self;
        loop {
            driver.poll()?;
            match parent_channel.poll() {
                Ok(Some(request)) => parent_channel.reply(&driver.command(&request, true))?,
                Ok(None) => (),
                Err(_) => parent_channel.closed = true,
            }
            if parent_channel.closed {
                driver.retire();
                os::close(&mut parent_channel.stream)?;
            }
            // One connection total, including unauthenticated handshakes.
            if client.is_none() {
                if let Some(stream) = listener.accept()? {
                    *client = Some((Channel::new(stream), false, Instant::now()));
                }
            }
            if let Some((channel, authenticated, since)) = client {
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
                    driver.client_attached = false;
                    if os::close(&mut channel.stream)? {
                        *client = None;
                    }
                }
            }
            // A lost client link alone preserves healthy speech. Once both owners
            // are gone, retire and exit; no missing reply becomes cleanup evidence.
            if parent_channel.closed
                && client.is_none()
                && driver.tree_retired()
                && driver.io_retired
            {
                return Ok(());
            }
            readiness::wait(driver, parent_channel, client.as_ref(), listener)?;
        }
    }
}
/// A private native stdio bridge carries bounded lifecycle frames, never speech.
pub fn bridge() -> Result<()> {
    let mut input = BufReader::new(io::stdin());
    let endpoint: GuardEndpoint = serde_json::from_value(wire::read_reply(&mut input)?)?;
    anyhow::ensure!(
        endpoint.version == 1 && endpoint.transport == os::TRANSPORT,
        "unsupported guard endpoint"
    );
    os::bridge(endpoint, input)
}
