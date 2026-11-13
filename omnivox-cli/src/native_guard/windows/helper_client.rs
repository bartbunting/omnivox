//! Worker end of the inherited creation link. Only initialization and helper
//! recovery use this mutex; speech and PCM retain their existing direct pipes.
use super::{helper_wire as wire, os, wire::TIMEOUT};
use omnivox_tts::{engine_configuration::LaunchEnvironment, native_helper::Child};
use std::ffi::OsString;
use std::io::{self, BufReader, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

struct Client {
    connection: BufReader<os::Blocking>,
    sequence: u64,
    synchronized: bool,
}
static CLIENT: OnceLock<Mutex<Option<Client>>> = OnceLock::new();

pub(crate) fn install(argument: &str) -> anyhow::Result<()> {
    // Validation duplicates the inherited object first, exactly as guard bootstrap.
    // This entry runs before files, engine threads or possible handle reuse.
    let handle = os::native::receive_one(argument)?;
    let client = Client {
        connection: BufReader::new(os::Blocking::from_handle(handle)?),
        sequence: 0,
        synchronized: true,
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
    let mutex = CLIENT
        .get()
        .ok_or_else(|| io::Error::other("native helper channel unavailable"))?;
    let mut state = creation_slot(mutex, TIMEOUT)?;
    let client = state
        .as_mut()
        .ok_or_else(|| io::Error::other("native helper ownership is unavailable"))?;
    let result = client.launch(engine, identity);
    if result.is_err() && !client.synchronized {
        // No reconnect or blind creation retry: an undelivered offer can contain
        // handles already installed in this process. The guard retires the worker
        // on link failure to close those actual handles, not guessed integers.
        let failed = state.take();
        drop(state);
        drop(failed);
    }
    result
}

fn creation_slot<T>(mutex: &Mutex<T>, phase_timeout: Duration) -> io::Result<MutexGuard<'_, T>> {
    // Waiting transfers no handles. Give the bounded helper set time to pass
    // through both creation and acknowledgement ahead of this caller. A single
    // phase deadline here falsely excludes healthy engines during a slow start.
    // Native I/O, creation and handoff retain their own unchanged deadlines;
    // the independent guard can still retire the worker during this wait.
    let deadline = Instant::now() + phase_timeout * (2 * super::windows_plan::MAX_HELPERS as u32);
    loop {
        match mutex.try_lock() {
            Ok(state) => return Ok(state),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "native helper creation queue timed out",
                ))
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(io::Error::other("native helper creation channel poisoned"))
            }
        }
    }
}

impl Client {
    fn send(&mut self, request: &wire::Request) -> io::Result<()> {
        serde_json::to_writer(self.connection.get_mut(), request).map_err(io::Error::other)?;
        self.connection.get_mut().write_all(b"\n")
    }
    fn receive(&mut self, id: u64) -> io::Result<wire::Reply> {
        let value = super::wire::read_reply(&mut self.connection).map_err(io::Error::other)?;
        let reply: wire::Reply = serde_json::from_value(value).map_err(io::Error::other)?;
        if reply.request_id != id {
            return Err(io::Error::other("uncorrelated native helper response"));
        }
        Ok(reply)
    }
    fn launch(&mut self, engine: &str, identity: String) -> io::Result<Child> {
        self.synchronized = false;
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
        let reply = self.receive(id)?;
        if reply.state == "rejected" && reply.handles.is_none() {
            if let Some(error) = &reply.error {
                // A correlated rejection transferred nothing. Keep this same
                // channel usable for future recovery; existing speech continues.
                self.synchronized = true;
                return Err(io::Error::other(error.clone()));
            }
        }
        if reply.state != "offered" || reply.error.is_some() {
            return Err(io::Error::other("invalid native helper offer"));
        }
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
        let reply = self.receive(id)?;
        if reply.state != "started" || reply.error.is_some() || reply.handles.is_some() {
            return Err(io::Error::other("invalid native helper start confirmation"));
        }
        self.synchronized = true;
        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_guard::wire::{Channel, WaitFor};

    #[test]
    fn serialized_helpers_do_not_share_one_creation_deadline() {
        let slot = Mutex::new(0);
        let count = super::super::windows_plan::MAX_HELPERS;
        let ready = std::sync::Barrier::new(count);
        let phase = Duration::from_millis(100);
        let started = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..count {
                scope.spawn(|| {
                    ready.wait();
                    let mut slot = creation_slot(&slot, phase).unwrap();
                    std::thread::sleep(phase / 2);
                    *slot += 1;
                });
            }
        });
        assert_eq!(*slot.lock().unwrap(), count);
        assert!(started.elapsed() > phase);
    }

    #[test]
    fn creation_queue_still_has_a_deadline() {
        let slot = Mutex::new(());
        let held = slot.lock().unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert_eq!(
                        creation_slot(&slot, Duration::from_millis(2))
                            .unwrap_err()
                            .kind(),
                        io::ErrorKind::TimedOut
                    );
                })
                .join()
                .unwrap();
        });
        drop(held);
    }

    #[test]
    fn correlated_rejection_keeps_the_creation_link_usable() {
        let (server, client) = os::channel_pair().unwrap();
        let thread = std::thread::spawn(move || {
            let mut channel = Channel::new(server);
            for expected in [1, 2] {
                let deadline = Instant::now() + TIMEOUT;
                let request = loop {
                    assert!(Instant::now() < deadline);
                    if let Some(request) = channel.poll_as::<wire::Request>().unwrap() {
                        break request;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                };
                assert_eq!(request.request_id, expected);
                channel
                    .reply(&wire::Reply {
                        request_id: expected,
                        state: "rejected".into(),
                        handles: None,
                        error: Some("helper unavailable".into()),
                    })
                    .unwrap();
                while matches!(channel.wait_for(), WaitFor::Write) {
                    assert!(Instant::now() < deadline);
                    channel.poll_as::<wire::Request>().unwrap();
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        let mut client = Client {
            connection: BufReader::new(os::Blocking::from_handle(client).unwrap()),
            sequence: 0,
            synchronized: true,
        };
        for _ in 0..2 {
            assert!(client.launch("flite", "test".into()).is_err());
            assert!(client.synchronized);
        }
        thread.join().unwrap();
    }
}
