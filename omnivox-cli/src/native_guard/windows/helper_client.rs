//! Worker end of the inherited creation link. Only initialization and helper
//! recovery use this mutex; speech and PCM retain their existing direct pipes.
use super::{helper_wire as wire, os, wire::TIMEOUT};
use omnivox_tts::{engine_configuration::LaunchEnvironment, native_helper::Child};
use std::ffi::OsString;
use std::io::{self, BufReader, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

struct Client {
    connection: BufReader<os::Blocking>,
    sequence: u64,
}
static CLIENT: OnceLock<Mutex<Option<Client>>> = OnceLock::new();

pub(crate) fn install(argument: &str) -> anyhow::Result<()> {
    // Validation duplicates the inherited object first, exactly as guard bootstrap.
    // This entry runs before files, engine threads or possible handle reuse.
    let handle = os::native::receive_one(argument)?;
    let client = Client {
        connection: BufReader::new(os::Blocking::from_handle(handle)?),
        sequence: 0,
    };
    anyhow::ensure!(
        CLIENT.set(Mutex::new(Some(client))).is_ok(),
        "duplicate native helper client"
    );
    omnivox_tts::native_helper::install(launch)?;
    Ok(())
}

fn launch(
    engine: &str,
    program: &Path,
    arguments: &[OsString],
    environment: &LaunchEnvironment,
) -> io::Result<Child> {
    let identity = wire::identity(engine, program, arguments, environment)?;
    let deadline = Instant::now() + TIMEOUT;
    let mutex = CLIENT
        .get()
        .ok_or_else(|| io::Error::other("native helper channel unavailable"))?;
    let mut state = loop {
        match mutex.try_lock() {
            Ok(state) => break state,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            _ => {
                return Err(io::Error::other(
                    "native helper creation channel busy or unavailable",
                ))
            }
        }
    };
    let client = state
        .as_mut()
        .ok_or_else(|| io::Error::other("native helper ownership is unavailable"))?;
    let result = client.launch(engine, identity);
    if result.is_err() {
        // No reconnect or blind creation retry: an undelivered offer can contain
        // handles already installed in this process. The guard retires the worker
        // on link failure to close those actual handles, not guessed integers.
        let failed = state.take();
        drop(state);
        drop(failed);
    }
    result
}

impl Client {
    fn send(&mut self, request: &wire::Request) -> io::Result<()> {
        serde_json::to_writer(self.connection.get_mut(), request).map_err(io::Error::other)?;
        self.connection.get_mut().write_all(b"\n")
    }
    fn receive(&mut self, id: u64, expected: &str) -> io::Result<wire::Reply> {
        let value = super::wire::read_reply(&mut self.connection).map_err(io::Error::other)?;
        let reply: wire::Reply = serde_json::from_value(value).map_err(io::Error::other)?;
        if reply.request_id != id || reply.state != expected || reply.error.is_some() {
            return Err(io::Error::other(
                reply
                    .error
                    .unwrap_or_else(|| "uncorrelated native helper response".into()),
            ));
        }
        Ok(reply)
    }
    fn launch(&mut self, engine: &str, identity: String) -> io::Result<Child> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("native helper sequence exhausted"))?;
        let id = self.sequence;
        self.send(&wire::Request {
            request_id: id,
            command: "create".into(),
            engine: engine.into(),
            launch: identity,
        })?;
        let reply = self.receive(id, "offered")?;
        let values = reply
            .handles
            .ok_or_else(|| io::Error::other("missing native helper handles"))?;
        if values.iter().any(|v| *v == 0 || *v == usize::MAX)
            || values[0] == values[1]
            || values[0] == values[2]
            || values[1] == values[2]
        {
            return Err(io::Error::other("invalid native helper handoff"));
        }
        // SAFETY: one correlated offer on the exclusively inherited guard link
        // transfers three distinct fresh handles once. There is no replay or
        // reconnection. The guard retains each actual process and targets this
        // actual worker handle when duplicating them.
        let [process, input, output] =
            values.map(|value| unsafe { OwnedHandle::from_raw_handle(value as _) });
        let child = Child::guarded(process, input, output)?;
        self.send(&wire::Request {
            request_id: id,
            command: "acknowledge".into(),
            engine: String::new(),
            launch: String::new(),
        })?;
        let reply = self.receive(id, "started")?;
        if reply.handles.is_some() {
            return Err(io::Error::other("duplicate native handle offer"));
        }
        Ok(child)
    }
}
