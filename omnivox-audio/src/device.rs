//! Replaceable Windows device output. Native calls stay on one owned thread;
//! admission uses short queue locks and never waits for a device to consume PCM.

#[cfg(windows)]
mod windows;

use crate::cancellation::OutputLifetime;
use crate::output::AudioControl;
use crate::rodio_output::CanonicalQueue;
use crate::{AudioError, CancellationToken};
use rodio::queue::SourcesQueueInput;
use rodio::Source;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const EVENT_CAPACITY: usize = 32;
const RETRY_DELAY: Duration = Duration::from_millis(250);
const AUTOMATIC_ATTEMPTS: usize = 4;

#[derive(Debug)]
enum Event {
    DefaultChanged,
    EndpointChanged(String),
    Rescan,
    Retry,
    Shutdown,
}

struct Signals {
    sender: mpsc::SyncSender<Event>,
    revision: AtomicU64,
    overflow: AtomicBool,
    retry_requested: AtomicBool,
    closed: AtomicBool,
}

impl Signals {
    fn notify(&self, event: Event) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        if let Err(mpsc::TrySendError::Full(_)) = self.sender.try_send(event) {
            self.overflow.store(true, Ordering::Release);
        }
    }

    fn retry(&self) {
        if !self.closed.load(Ordering::Acquire)
            && !self.retry_requested.swap(true, Ordering::AcqRel)
        {
            // A full channel already guarantees that the owner will wake.
            let _ = self.sender.try_send(Event::Retry);
        }
    }
}

#[derive(Default)]
struct Pending {
    count: Mutex<usize>,
    changed: Condvar,
}

impl Pending {
    fn wait(&self) {
        let mut count = self.count.lock().unwrap();
        while *count != 0 {
            count = self.changed.wait(count).unwrap();
        }
    }
}

struct CountedSource {
    inner: Box<dyn Source<Item = f32> + Send>,
    lifetime: OutputLifetime,
    pending: Arc<Pending>,
}

impl Iterator for CountedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.lifetime.is_cancelled() {
            return None;
        }
        let sample = self.inner.next();
        if self.lifetime.is_cancelled() {
            None
        } else {
            sample
        }
    }
}

impl Source for CountedSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

impl Drop for CountedSource {
    fn drop(&mut self) {
        // Source Drop publishes its playback ticket before drain is released.
        let source = std::mem::replace(
            &mut self.inner,
            Box::new(rodio::source::Empty::<f32>::new()),
        );
        drop(source);
        let mut count = self.pending.count.lock().unwrap();
        *count -= 1;
        self.pending.changed.notify_all();
    }
}

struct QueueConnection {
    input: Arc<SourcesQueueInput<f32>>,
    generation: Arc<AtomicU64>,
    closed: CancellationToken,
    pending: Arc<Pending>,
}

impl QueueConnection {
    fn new() -> (Self, CanonicalQueue) {
        let (input, output) = rodio::queue::queue(true);
        (
            Self {
                input,
                generation: Arc::new(AtomicU64::new(0)),
                closed: CancellationToken::new(),
                pending: Arc::new(Pending::default()),
            },
            CanonicalQueue::new(output),
        )
    }

    fn lifetime(&self) -> OutputLifetime {
        OutputLifetime {
            generation: self.generation.clone(),
            expected: self.generation.load(Ordering::Acquire),
            closed: self.closed.clone(),
        }
    }

    fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.input.clear();
        self.pending = Arc::new(Pending::default());
    }
}

impl Drop for QueueConnection {
    fn drop(&mut self) {
        self.closed.cancel();
        self.input.set_keep_alive_if_empty(false);
        self.input.clear();
    }
}

pub(crate) struct DeviceSink {
    connection: Mutex<Option<QueueConnection>>,
    signals: Arc<Signals>,
}

impl DeviceSink {
    pub(crate) fn append<S: Source<Item = f32> + Send + 'static>(
        &self,
        source: S,
    ) -> Result<(), AudioError> {
        self.append_with(|_| Box::new(source))
    }

    pub(crate) fn append_with<F>(&self, make_source: F) -> Result<(), AudioError>
    where
        F: FnOnce(OutputLifetime) -> Box<dyn Source<Item = f32> + Send>,
    {
        let connection = self.connection.lock().unwrap();
        let Some(queue) = connection
            .as_ref()
            .filter(|_| !self.signals.closed.load(Ordering::Acquire))
        else {
            self.signals.retry();
            return Err(AudioError::PlaybackError(
                "Windows default output is unavailable; this audio was discarded".into(),
            ));
        };
        let lifetime = queue.lifetime();
        let source = make_source(lifetime.clone());
        *queue.pending.count.lock().unwrap() += 1;
        queue.input.append(CountedSource {
            inner: source,
            lifetime,
            pending: queue.pending.clone(),
        });
        Ok(())
    }

    pub(crate) fn clear(&self) {
        if let Some(queue) = self.connection.lock().unwrap().as_mut() {
            queue.clear();
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.connection
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |queue| *queue.pending.count.lock().unwrap())
    }

    pub(crate) fn drain(&self) {
        let pending = self
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .map(|queue| queue.pending.clone());
        if let Some(pending) = pending {
            pending.wait();
        }
    }

    fn replace(&self, connection: Option<QueueConnection>) {
        let old = std::mem::replace(&mut *self.connection.lock().unwrap(), connection);
        drop(old);
    }
}

trait Backend {
    type Connection;
    fn default_device(&mut self) -> Result<Option<String>, AudioError>;
    fn open(&mut self) -> Result<(Self::Connection, [QueueConnection; 3]), AudioError>;
}

struct Owner<B: Backend> {
    backend: B,
    connection: Option<B::Connection>,
    device: Option<String>,
    sinks: [Arc<DeviceSink>; 3],
    control: Arc<AudioControl>,
    signals: Arc<Signals>,
}

impl<B: Backend> Owner<B> {
    fn retire(&mut self) {
        self.control.interrupt_output(|| {
            for sink in &self.sinks {
                sink.replace(None);
            }
        });
        self.device = None;
        // Never hold an admission gate across native teardown.
        self.connection = None;
    }

    fn reconcile(&mut self, force: bool) -> Result<bool, AudioError> {
        let revision = self.signals.revision.load(Ordering::Acquire);
        let target = self.backend.default_device();
        if !force && self.connection.is_some() && target.as_ref().ok() == Some(&self.device) {
            return Ok(true);
        }
        self.retire();
        let target = target?.ok_or_else(|| {
            AudioError::DeviceNotFound("Windows has no default playback endpoint".into())
        })?;
        let (connection, queues) = self.backend.open()?;
        if self.signals.closed.load(Ordering::Acquire)
            || self.signals.revision.load(Ordering::Acquire) != revision
            || self.backend.default_device()?.as_deref() != Some(target.as_str())
        {
            // Queues have never admitted samples, and this candidate cannot
            // overwrite a newer device choice or survive shutdown.
            drop(queues);
            drop(connection);
            return Ok(false);
        }
        let mut published = false;
        self.control.interrupt_output(|| {
            // The gates may have been busy since the native checks above.
            if self.signals.closed.load(Ordering::Acquire)
                || self.signals.revision.load(Ordering::Acquire) != revision
            {
                return;
            }
            for (sink, queue) in self.sinks.iter().zip(queues) {
                sink.replace(Some(queue));
            }
            published = true;
        });
        if !published {
            return Ok(false);
        }
        self.connection = Some(connection);
        self.device = Some(target);
        tracing::info!(lifecycle_stage = "output_device_ready", device_id = ?self.device,
            "Following Windows default audio output");
        Ok(true)
    }

    fn run(&mut self, receiver: mpsc::Receiver<Event>) {
        let mut retries = 0;
        let mut force = false;
        let mut next_attempt = Instant::now();
        while !self.signals.closed.load(Ordering::Acquire) {
            let event = if retries > 0 {
                match receiver.recv_timeout(next_attempt.saturating_duration_since(Instant::now()))
                {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => Event::Retry,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match receiver.recv() {
                    Ok(event) => event,
                    Err(_) => break,
                }
            };
            force |= self.signals.overflow.swap(false, Ordering::AcqRel);
            let mut fresh = false;
            for event in std::iter::once(event).chain(receiver.try_iter().take(EVENT_CAPACITY)) {
                match event {
                    Event::Shutdown => return,
                    Event::DefaultChanged | Event::Rescan => fresh = true,
                    Event::EndpointChanged(id) => {
                        force |= self.device.as_ref() == Some(&id);
                        fresh = true;
                    }
                    Event::Retry => {}
                }
            }
            fresh |= self.signals.retry_requested.swap(false, Ordering::AcqRel);
            if fresh {
                retries = AUTOMATIC_ATTEMPTS;
            }
            if self.signals.closed.load(Ordering::Acquire) {
                break;
            }
            if Instant::now() < next_attempt {
                continue;
            }
            match self.reconcile(force) {
                Ok(true) => retries = 0,
                Ok(false) => retries = retries.max(1),
                Err(error) => {
                    tracing::warn!(lifecycle_stage = "output_device_unavailable", %error,
                        "Output unavailable; interrupted audio will not be replayed");
                    retries = retries.saturating_sub(1);
                }
            }
            force = false;
            next_attempt = Instant::now() + RETRY_DELAY;
        }
    }
}

impl<B: Backend> Drop for Owner<B> {
    fn drop(&mut self) {
        self.signals.closed.store(true, Ordering::Release);
        self.retire();
    }
}

pub(crate) struct DeviceRuntime {
    signals: Arc<Signals>,
    worker: Option<JoinHandle<()>>,
}

impl DeviceRuntime {
    #[cfg(windows)]
    pub(crate) fn new(depths: [usize; 3]) -> Result<(Self, Arc<AudioControl>), AudioError> {
        Self::start(depths, windows::WindowsBackend::new)
    }

    fn start<B, F>(depths: [usize; 3], factory: F) -> Result<(Self, Arc<AudioControl>), AudioError>
    where
        B: Backend + 'static,
        F: FnOnce(Arc<Signals>) -> Result<B, AudioError> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
        let signals = Arc::new(Signals {
            sender,
            revision: AtomicU64::new(0),
            overflow: AtomicBool::new(false),
            retry_requested: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let sinks = std::array::from_fn(|_| {
            Arc::new(DeviceSink {
                connection: Mutex::new(None),
                signals: signals.clone(),
            })
        });
        let control = AudioControl::following_device(sinks.clone(), depths);
        let owner_control = control.clone();
        let owner_signals = signals.clone();
        let (started, startup) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("omnivox-device-output".into())
            .spawn(move || {
                let backend = match factory(owner_signals.clone()) {
                    Ok(backend) => backend,
                    Err(error) => {
                        let _ = started.send(Err(error));
                        return;
                    }
                };
                let mut owner = Owner {
                    backend,
                    connection: None,
                    device: None,
                    sinks,
                    control: owner_control,
                    signals: owner_signals,
                };
                for _ in 0..AUTOMATIC_ATTEMPTS {
                    match owner.reconcile(true) {
                        Ok(true) => {
                            if started.send(Ok(())).is_ok() {
                                owner.run(receiver);
                            }
                            return;
                        }
                        Ok(false) => continue,
                        Err(error) => {
                            let _ = started.send(Err(error));
                            return;
                        }
                    }
                }
                let _ = started.send(Err(AudioError::DeviceNotFound(
                    "default output kept changing during startup".into(),
                )));
            })
            .map_err(|error| AudioError::PlaybackError(format!("device output thread: {error}")))?;
        let runtime = Self {
            signals,
            worker: Some(worker),
        };
        startup.recv().map_err(|_| {
            AudioError::PlaybackError("device output thread exited during startup".into())
        })??;
        Ok((runtime, control))
    }
}

impl Drop for DeviceRuntime {
    fn drop(&mut self) {
        self.signals.closed.store(true, Ordering::Release);
        let _ = self.signals.sender.try_send(Event::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
