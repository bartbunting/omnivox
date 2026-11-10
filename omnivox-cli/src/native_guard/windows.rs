//! Retained Windows identities and overlapped local control. Speech stays on
//! anonymous pipes; neither a PID nor a pipe disconnect establishes cleanup.
#[path = "windows/native.rs"]
pub(super) mod native;
#[path = "windows/pipe.rs"]
mod pipe;
use super::wire::{MAX_FRAME, TIMEOUT};
pub(super) use native::Process as GuardChild;
use omnivox_tts::voice_library::local::{self, GuardEndpoint};
pub(super) use pipe::{Listener, Stream as Control};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsHandle, AsRawHandle, OwnedHandle};
use std::sync::Arc;
use std::time::{Duration, Instant};
pub type Parent = OwnedHandle;
pub type Completion = OwnedHandle;
pub const TRANSPORT: &str = "windows-pipe";

pub struct Lease(native::Event);
impl Drop for Lease {
    fn drop(&mut self) {
        // Only the final actual speech-I/O owner signals. Failure leaves proof
        // false; the retained process handle can still prove complete owner exit.
        let _ = self.0.signal();
    }
}
pub struct Blocking {
    stream: Control,
    timeout: Option<Duration>,
}
impl Blocking {
    fn retry<T>(
        &mut self,
        writing: bool,
        mut op: impl FnMut(&mut Control) -> io::Result<T>,
    ) -> io::Result<T> {
        let deadline = self.timeout.map(|duration| Instant::now() + duration);
        loop {
            match op(&mut self.stream) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let event = if writing {
                        self.stream.write_event()
                    } else {
                        self.stream.read_event()
                    }
                    .ok_or_else(|| io::Error::other("missing pending native control operation"))?;
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    native::wait(&[event], millis(deadline))?;
                }
                result => return result,
            }
        }
    }
}
impl Read for Blocking {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.retry(false, |s| s.read(bytes))
    }
}
impl Write for Blocking {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.retry(true, |s| s.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.retry(true, Write::flush)
    }
}
pub fn timeouts(stream: &mut Blocking, timeout: Option<Duration>) -> io::Result<()> {
    stream.timeout = timeout;
    Ok(())
}
pub fn into_channel(stream: Blocking) -> io::Result<Control> {
    Ok(stream.stream)
}
pub fn ended(parent: &Parent) -> io::Result<bool> {
    native::signalled(parent.as_handle())
}
pub fn terminate_parent(parent: &Parent) -> io::Result<()> {
    native::terminate(parent.as_handle())
}
pub fn completed(completion: &mut Completion, parent: &Parent) -> io::Result<bool> {
    // The inherited completion observer cannot signal. Abrupt process death
    // closes all its handles and retires its synchronous anonymous-pipe I/O.
    Ok(native::signalled(completion.as_handle())? || ended(parent)?)
}
pub fn close(stream: &mut Control) -> io::Result<bool> {
    stream.close()?;
    stream.poll_closed()
}
pub struct Spawned {
    pub child: GuardChild,
    pub control: Blocking,
    pub input: File,
    pub output: File,
    pub completion: Arc<Lease>,
}
pub fn launch(executable: &str) -> io::Result<Spawned> {
    let id = local::new_uuid().map_err(io::Error::other)?;
    let mut listener = Listener::bind(&id)?;
    let control = connect(listener.address())?;
    let inherited_control = loop {
        if let Some(stream) = listener.accept()? {
            break stream.into_handle()?;
        }
        std::thread::sleep(super::TICK);
    };
    let (inherited_input, input) = native::speech_pipe()?;
    let (output, inherited_output) = native::speech_pipe()?;
    let stderr = std::io::stderr().as_handle().try_clone_to_owned()?;
    let completion = Arc::new(Lease(native::Event::new()?));
    let observer = completion.0.observer()?;
    let parent = native::current_process()?;
    let child = native::spawn(
        std::path::Path::new(executable),
        [
            inherited_control.as_handle(),
            inherited_input.as_handle(),
            inherited_output.as_handle(),
            stderr.as_handle(),
            observer.as_handle(),
            parent.as_handle(),
        ],
    )?;
    Ok(Spawned {
        child,
        control: Blocking {
            stream: control,
            timeout: Some(TIMEOUT),
        },
        input,
        output,
        completion,
    })
}
/// # Safety
/// The private executable entry transfers these six inherited objects once,
/// before starting threads or opening resources that could reuse handle values.
pub unsafe fn receive(
    arguments: &[String],
) -> io::Result<(Blocking, File, File, File, Completion, Parent)> {
    let [control, input, output, error, completion, parent] = native::receive(arguments)?;
    Ok((
        Blocking {
            stream: Control::new(control)?,
            timeout: Some(TIMEOUT),
        },
        input.into(),
        output.into(),
        error.into(),
        completion,
        parent,
    ))
}
fn connect(address: &str) -> io::Result<Control> {
    let mut connector = pipe::Connector::new(address, Instant::now() + TIMEOUT)?;
    loop {
        if let Some(stream) = connector.poll(Instant::now())? {
            return Ok(stream);
        }
        std::thread::sleep(super::TICK);
    }
}
pub(super) fn millis(deadline: Option<Instant>) -> u32 {
    deadline.map_or(u32::MAX, |d| {
        let duration = d.saturating_duration_since(Instant::now());
        duration
            .as_millis()
            .saturating_add(u128::from(
                !duration.subsec_nanos().is_multiple_of(1_000_000),
            ))
            .min((u32::MAX - 1) as u128) as u32
    })
}

/// This bridge owns no native worker. A bounded input channel lets its native
/// thread cancel overlapped control I/O promptly when actual stdin closes.
pub fn bridge(endpoint: GuardEndpoint, mut input: BufReader<io::Stdin>) -> anyhow::Result<()> {
    anyhow::ensure!(
        endpoint.address == format!(r"\\.\pipe\omnivox-guard-{}", endpoint.guard),
        "invalid guard address"
    );
    let mut stream = connect(&endpoint.address)?;
    let ready = Arc::new(native::Event::auto()?);
    let notify = Arc::clone(&ready);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("guard-control-input".into())
        .spawn(move || {
            let result: anyhow::Result<()> = (|| loop {
                let mut line = Vec::new();
                (&mut input)
                    .take((MAX_FRAME + 1) as u64)
                    .read_until(b'\n', &mut line)?;
                if line.is_empty() {
                    return Ok(());
                }
                anyhow::ensure!(
                    line.len() <= MAX_FRAME && line.last() == Some(&b'\n'),
                    "guard bridge input exceeds bound"
                );
                sender.send(line)?;
                notify.signal()?;
            })();
            drop(sender);
            let _ = notify.signal();
            if let Err(error) = result {
                eprintln!("Guard control input ended: {error}");
            }
        })?;
    let mut pending: Option<(Vec<u8>, usize)> = None;
    let mut output = io::stdout().lock();
    let mut bytes = [0; MAX_FRAME];
    loop {
        if pending.is_none() {
            match receiver.try_recv() {
                Ok(line) => pending = Some((line, 0)),
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if let Some((line, offset)) = &mut pending {
            match stream.write(&line[*offset..]) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    *offset += n;
                    if *offset == line.len() {
                        pending = None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                Err(e) => return Err(e.into()),
            }
        }
        match stream.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                output.write_all(&bytes[..n])?;
                output.flush()?;
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
            Err(e) => return Err(e.into()),
        }
        // Check queued input again after a complete write. It may have arrived
        // before the previous notification was consumed by a native wait.
        if pending.is_none() {
            match receiver.try_recv() {
                Ok(line) => {
                    pending = Some((line, 0));
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
            }
        }
        let mut events = vec![ready.handle().as_raw_handle()];
        events.extend(stream.read_event());
        events.extend(stream.write_event());
        native::wait(&events, u32::MAX)?;
    }
}

#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;
