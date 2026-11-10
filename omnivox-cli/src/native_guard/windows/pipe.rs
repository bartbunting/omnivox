//! Private Windows pipes for independent native cleanup control.
//!
//! Polling never waits for a peer or native I/O completion. Each direction owns
//! one bounded buffer and one stable OVERLAPPED allocation. `close` requests
//! cancellation and `poll_closed` proves completion; callers must retain this
//! owner until then. Drop waits as a memory-safety fallback, so a future session
//! polling loop must explicitly retire it before dropping it. Neither pipe EOF
//! nor completed cancellation proves worker/job retirement.
use std::cell::UnsafeCell;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::Instant;

#[path = "pipe/native.rs"]
mod native;
#[path = "pipe/security.rs"]
mod security;
use native::*;

const BUFFER: usize = 16 * 1024;
// One connected control client and one continuously retained accepting instance.
const MAX_INSTANCES: u32 = 2;
const PREFIX: &str = r"\\.\pipe\omnivox-guard-";

pub fn valid_address(address: &str) -> bool {
    address.strip_prefix(PREFIX).is_some_and(|id| {
        id.len() == 36
            && id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                }
            })
    })
}
fn would_block() -> io::Error {
    io::ErrorKind::WouldBlock.into()
}

struct Operation {
    overlapped: Box<UnsafeCell<Overlapped>>,
    event: OwnedHandle,
    pending: bool,
}
// SAFETY: event/OVERLAPPED have sole ownership, the allocation never moves, and
// native access ends before destruction. Operations require exclusive access;
// moving this owner to another thread does not move native storage.
unsafe impl Send for Operation {}
impl Operation {
    fn new() -> io::Result<Self> {
        // SAFETY: unnamed, non-inheritable manual-reset event.
        let event = unsafe { owned(CreateEventW(null(), 1, 0, null()))? };
        Ok(Self {
            overlapped: Box::new(UnsafeCell::new(Overlapped {
                event: event.as_raw_handle(),
                ..Default::default()
            })),
            event,
            pending: false,
        })
    }
    fn reset(&mut self) -> io::Result<()> {
        assert!(!self.pending);
        // SAFETY: owned event; previous I/O has completed before reuse.
        check(unsafe { ResetEvent(self.event.as_raw_handle()) })?;
        // SAFETY: no native access remains after completion. UnsafeCell permits
        // the kernel to update the status fields while a later poll is pending.
        unsafe {
            *self.overlapped.get() = Overlapped {
                event: self.event.as_raw_handle(),
                ..Default::default()
            }
        };
        Ok(())
    }
    fn issued(&mut self, result: i32) -> io::Result<()> {
        if result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(PENDING) {
                return Err(error);
            }
        }
        // Also collect immediate completion using GetOverlappedResult.
        self.pending = true;
        Ok(())
    }
    fn finish(&mut self, handle: Handle, wait: bool) -> io::Result<Option<usize>> {
        assert!(self.pending);
        let mut count = 0;
        // SAFETY: handle, stable operation and its buffer remain owned until
        // completion. The polling path always supplies FALSE for wait.
        if unsafe {
            GetOverlappedResult(handle, self.overlapped.get(), &mut count, i32::from(wait))
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(INCOMPLETE) {
                return Ok(None);
            }
            self.pending = false;
            return Err(error);
        }
        self.pending = false;
        Ok(Some(count as usize))
    }
    fn cancel(&mut self, handle: Handle) -> io::Result<()> {
        if self.pending {
            // SAFETY: only this owner's operation is cancelled; its storage is
            // retained even if cancellation succeeds or races with completion.
            if unsafe { CancelIoEx(handle, self.overlapped.get()) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(NOT_FOUND) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

/// A connected duplex channel, with independent bounded read/write operations.
/// Write retries must keep the same prefix until the returned byte count advances.
pub struct Stream {
    handle: Option<OwnedHandle>,
    connect: Operation,
    read: Operation,
    write: Operation,
    input: Box<UnsafeCell<[u8; BUFFER]>>,
    output: Box<[u8; BUFFER]>,
    start: usize,
    end: usize,
    writing: usize,
    eof: bool,
    closing: bool,
}
impl Stream {
    pub(super) fn new(handle: OwnedHandle) -> io::Result<Self> {
        Ok(Self {
            handle: Some(handle),
            connect: Operation::new()?,
            read: Operation::new()?,
            write: Operation::new()?,
            input: Box::new(UnsafeCell::new([0; BUFFER])),
            output: Box::new([0; BUFFER]),
            start: 0,
            end: 0,
            writing: 0,
            eof: false,
            closing: false,
        })
    }
    fn handle(&self) -> io::Result<Handle> {
        if self.closing || self.connect.pending {
            return Err(io::ErrorKind::NotConnected.into());
        }
        self.handle
            .as_ref()
            .map(AsRawHandle::as_raw_handle)
            .ok_or_else(|| io::ErrorKind::NotConnected.into())
    }
    fn listen(&mut self) -> io::Result<()> {
        let raw = self.handle.as_ref().unwrap().as_raw_handle();
        // SAFETY: live pipe and stable operation; a client can connect between
        // CreateNamedPipe and this call, yielding PIPE_CONNECTED without I/O.
        let result = unsafe { ConnectNamedPipe(raw, self.connect.overlapped.get()) };
        if result == 0 && io::Error::last_os_error().raw_os_error() == Some(PIPE_CONNECTED) {
            return Ok(());
        }
        self.connect.issued(result)
    }
    fn connected(&mut self) -> io::Result<bool> {
        if self.connect.pending {
            let raw = self
                .handle
                .as_ref()
                .ok_or(io::ErrorKind::NotConnected)?
                .as_raw_handle();
            Ok(self.connect.finish(raw, false)?.is_some())
        } else {
            Ok(true)
        }
    }

    /// Peek into retained read storage. Reading only through a handshake's LF
    /// leaves any coalesced speech bytes in this same owner for the next reader.
    pub fn peek(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let handle = self.handle()?;
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.start == self.end && !self.eof {
            if !self.read.pending {
                self.read.reset()?;
                // SAFETY: buffer and operation have stable addresses and stay
                // alive until finish, including during cancellation and Drop.
                let result = unsafe {
                    ReadFile(
                        handle,
                        self.input.get().cast(),
                        BUFFER as u32,
                        null_mut(),
                        self.read.overlapped.get(),
                    )
                };
                if let Err(error) = self.read.issued(result) {
                    if matches!(error.raw_os_error(), Some(BROKEN_PIPE | PIPE_NOT_CONNECTED)) {
                        self.eof = true;
                        return Ok(0);
                    }
                    return Err(error);
                }
            }
            match self.read.finish(handle, false) {
                Ok(Some(count)) => {
                    self.start = 0;
                    self.end = count;
                    self.eof = count == 0;
                }
                Ok(None) => return Err(would_block()),
                Err(e) if matches!(e.raw_os_error(), Some(BROKEN_PIPE | PIPE_NOT_CONNECTED)) => {
                    self.eof = true;
                }
                Err(e) => return Err(e),
            }
        }
        let count = bytes.len().min(self.end - self.start);
        // SAFETY: completion was observed before exposing any input bytes; no
        // new read starts until this retained tail has been consumed.
        let input = unsafe { &*self.input.get() };
        bytes[..count].copy_from_slice(&input[self.start..self.start + count]);
        Ok(count)
    }

    /// Seal admission and request cancellation. Success is not completion.
    pub fn close(&mut self) -> io::Result<()> {
        self.closing = true;
        if let Some(handle) = &self.handle {
            // Attempt all directions even if one cancellation fails.
            let mut result = Ok(());
            for operation in [&mut self.connect, &mut self.read, &mut self.write] {
                if let Err(e) = operation.cancel(handle.as_raw_handle()) {
                    result = Err(e);
                }
            }
            result?;
        }
        Ok(())
    }
    /// Keep polling after close, even after its error. Retain this owner when
    /// false: a timeout is not permission to free native I/O memory.
    pub fn poll_closed(&mut self) -> io::Result<bool> {
        if !self.closing {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let Some(handle) = &self.handle else {
            return Ok(true);
        };
        for operation in [&mut self.connect, &mut self.read, &mut self.write] {
            if operation.pending {
                // Any completed result, including ABORTED, retires this I/O.
                // finish retains pending state only for INCOMPLETE.
                let _ = operation.finish(handle.as_raw_handle(), false);
            }
        }
        if self.connect.pending || self.read.pending || self.write.pending {
            return Ok(false);
        }
        self.handle.take();
        Ok(true)
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.peek(bytes)?;
        self.start += count;
        Ok(count)
    }
}
impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let handle = self.handle()?;
        if self.write.pending {
            if !bytes.starts_with(&self.output[..self.writing]) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
        } else {
            if bytes.is_empty() {
                return Ok(0);
            }
            self.writing = bytes.len().min(BUFFER);
            self.output[..self.writing].copy_from_slice(&bytes[..self.writing]);
            self.write.reset()?;
            // SAFETY: retained buffer/operation, exclusively owned until finish.
            let result = unsafe {
                WriteFile(
                    handle,
                    self.output.as_ptr(),
                    self.writing as u32,
                    null_mut(),
                    self.write.overlapped.get(),
                )
            };
            self.write.issued(result)?;
        }
        self.write.finish(handle, false)?.ok_or_else(would_block)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.handle()?;
        // Never call FlushFileBuffers: it waits for the remote reader.
        if self.write.pending {
            Err(would_block())
        } else {
            Ok(())
        }
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.close();
        if let Some(handle) = &self.handle {
            for operation in [&mut self.connect, &mut self.read, &mut self.write] {
                while operation.pending {
                    let _ = operation.finish(handle.as_raw_handle(), true);
                }
            }
        }
    }
}

/// One accepting instance at a time. Closing retains its cancelled accept until
/// poll_closed confirms completion. Already accepted streams have their own owner.
pub struct Listener {
    address: String,
    name: Vec<u16>,
    security: security::Security,
    pending: Option<Stream>,
    listening: bool,
    closing: bool,
}
impl Listener {
    pub fn bind(id: &str) -> io::Result<Self> {
        let address = format!("{PREFIX}{id}");
        let name = wide(&address);
        let security = security::Security::current_user()?;
        // First-instance exclusivity: never adopt or disconnect an existing pipe.
        let pending = Some(Stream::new(security.pipe(&name, true)?)?);
        Ok(Self {
            address,
            name,
            security,
            pending,
            listening: false,
            closing: false,
        })
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn accept(&mut self) -> io::Result<Option<Stream>> {
        if self.closing {
            return Err(io::ErrorKind::NotConnected.into());
        }
        // Setup may run on a short-lived thread. Issue the asynchronous accept
        // on the polling thread: exiting its issuing thread can abort pending I/O.
        if !self.listening {
            self.pending
                .as_mut()
                .ok_or(io::ErrorKind::NotConnected)?
                .listen()?;
            self.listening = true;
        }
        if !self
            .pending
            .as_mut()
            .ok_or(io::ErrorKind::NotConnected)?
            .connected()?
        {
            return Ok(None);
        }
        // Create the next instance before releasing the current one, retaining
        // the namespace continuously. No disconnect/reuse of accepted channels.
        let next = Stream::new(self.security.pipe(&self.name, false)?)?;
        self.listening = false;
        Ok(self.pending.replace(next))
    }
    pub fn close(&mut self) -> io::Result<()> {
        self.closing = true;
        self.pending.as_mut().map_or(Ok(()), Stream::close)
    }
    #[cfg(test)]
    pub fn poll_closed(&mut self) -> io::Result<bool> {
        if !self.closing {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if let Some(stream) = &mut self.pending {
            if !stream.poll_closed()? {
                return Ok(false);
            }
        }
        self.pending.take();
        Ok(true)
    }
}

/// Bounded retry for an occupied local instance. The caller owns the deadline;
/// no blocking WaitNamedPipe, detached task or automatic reconnection is used.
pub struct Connector {
    name: Vec<u16>,
    deadline: Instant,
    done: bool,
}
impl Connector {
    pub fn new(address: &str, deadline: Instant) -> io::Result<Self> {
        if !valid_address(address) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            name: wide(address),
            deadline,
            done: false,
        })
    }
    pub fn poll(&mut self, now: Instant) -> io::Result<Option<Stream>> {
        if self.done {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if now >= self.deadline {
            self.done = true;
            return Err(io::ErrorKind::TimedOut.into());
        }
        match security::client(&self.name) {
            Ok(handle) => {
                self.done = true;
                Stream::new(handle).map(Some)
            }
            Err(e) if e.raw_os_error() == Some(PIPE_BUSY) => Ok(None),
            Err(e) => {
                self.done = true;
                Err(e)
            }
        }
    }
}

// SAFETY: every operation/buffer has stable sole ownership. All methods require
// exclusive access; moving Stream never moves native storage. It is not Sync.
unsafe impl Send for Stream {}
impl Stream {
    pub(super) fn into_handle(mut self) -> io::Result<OwnedHandle> {
        if self.connect.pending || self.read.pending || self.write.pending {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.handle
            .take()
            .ok_or_else(|| io::ErrorKind::NotConnected.into())
    }
    pub(crate) fn read_event(&self) -> Option<std::os::windows::io::RawHandle> {
        self.read.pending.then(|| self.read.event.as_raw_handle())
    }
    pub(crate) fn write_event(&self) -> Option<std::os::windows::io::RawHandle> {
        self.write.pending.then(|| self.write.event.as_raw_handle())
    }
    pub(crate) fn closing_events(&self, events: &mut Vec<std::os::windows::io::RawHandle>) {
        for op in [&self.connect, &self.read, &self.write] {
            if op.pending {
                events.push(op.event.as_raw_handle());
            }
        }
    }
}
impl Listener {
    pub(crate) fn event(&self) -> Option<std::os::windows::io::RawHandle> {
        let stream = self.pending.as_ref()?;
        (self.listening && stream.connect.pending).then(|| stream.connect.event.as_raw_handle())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
#[cfg(test)]
#[path = "pipe/tests.rs"]
mod tests;
