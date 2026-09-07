//! Opt-in native PulseAudio output. Each lane has an independent, persistent
//! stream and source consumer; libpulse mixes the lanes on the selected sink.

mod native;

use crate::buffer::{CHANNELS, SAMPLE_RATE};
use crate::{AudioError, CancellationToken};
use rodio::Source;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const WRITE_FRAMES: usize = SAMPLE_RATE as usize * 5 / 1000;
const BYTES_PER_FRAME: usize = CHANNELS as usize * std::mem::size_of::<f32>();
const POLL: Duration = Duration::from_millis(1);
const STALL_TIMEOUT: Duration = Duration::from_secs(3);
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

trait PlaybackDevice {
    fn writable_frames(&mut self) -> Result<usize, String>;
    fn write(&mut self, samples: &[f32]) -> Result<(), String>;
    fn cork(&mut self, corked: bool) -> Result<(), String>;
    fn flush(&mut self) -> Result<(), String>;
    fn active(&mut self, active: bool);
    fn begin_drain(&mut self) -> Result<(), String>;
    fn drained(&mut self) -> Result<bool, String>;
    fn cancel_drain(&mut self);
    fn report(&mut self);
}

pub(crate) struct SourceLifetime {
    generation: Arc<AtomicU64>,
    expected: u64,
    closed: CancellationToken,
}

impl SourceLifetime {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.closed.is_cancelled() || self.generation.load(Ordering::Acquire) != self.expected
    }
}

struct Queued {
    source: Box<dyn Source<Item = f32> + Send>,
    generation: u64,
}

#[derive(Default)]
struct Queue {
    sources: VecDeque<Queued>,
    pending: usize,
    idle: bool,
    failure: Option<String>,
    retry_at: Option<Instant>,
    connection_closed: CancellationToken,
    connected: bool,
}

struct State {
    queue: Mutex<Queue>,
    changed: Condvar,
    generation: Arc<AtomicU64>,
    closed: CancellationToken,
    shutdown: AtomicBool,
}

impl State {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(Queue {
                idle: true,
                ..Queue::default()
            }),
            changed: Condvar::new(),
            generation: Arc::new(AtomicU64::new(0)),
            closed: CancellationToken::new(),
            shutdown: AtomicBool::new(false),
        })
    }

    fn finish(&self, source: Queued) {
        drop(source); // Tickets/callback state are retired outside the queue lock.
        let mut queue = self.queue.lock().unwrap();
        queue.pending -= 1;
        self.changed.notify_all();
    }

    fn retire(&self, failure: Option<String>) {
        let sources = {
            let mut queue = self.queue.lock().unwrap();
            // Retire the old connection's producers before admitting another
            // generation. Shutdown has a separate, permanent lifetime.
            queue.connection_closed.cancel();
            queue.connection_closed = CancellationToken::new();
            queue.connected = false;
            self.generation.fetch_add(1, Ordering::AcqRel);
            queue.retry_at = failure.as_ref().map(|_| Instant::now() + RECONNECT_DELAY);
            queue.failure = failure;
            queue.pending = 0;
            queue.idle = true;
            std::mem::take(&mut queue.sources)
        };
        drop(sources);
        self.changed.notify_all();
    }
}

pub(crate) struct PulseSink {
    state: Arc<State>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

fn latency_ms(value: Option<&str>) -> Result<u32, AudioError> {
    match value {
        None => Ok(20),
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|value| (10..=200).contains(value))
            .ok_or_else(|| {
                AudioError::PlaybackError(
                    "OMNIVOX_PULSE_LATENCY_MS must be an integer from 10 through 200".into(),
                )
            }),
    }
}

impl PulseSink {
    pub(crate) fn new(name: &'static str) -> Result<Arc<Self>, AudioError> {
        if std::env::var_os("PULSE_LATENCY_MSEC").is_some() {
            return Err(AudioError::PlaybackError(
                "unset PULSE_LATENCY_MSEC for native PulseAudio; use OMNIVOX_PULSE_LATENCY_MS instead".into(),
            ));
        }
        let request = std::env::var("OMNIVOX_PULSE_LATENCY_MS")
            .map(Some)
            .or_else(|error| match error {
                std::env::VarError::NotPresent => Ok(None),
                _ => Err(AudioError::PlaybackError(
                    "OMNIVOX_PULSE_LATENCY_MS is not Unicode".into(),
                )),
            })?;
        let request = latency_ms(request.as_deref())?;
        Self::start(name, move |closed| {
            native::Client::open(name, request, closed)
        })
    }

    fn start<D: PlaybackDevice + 'static>(
        name: &'static str,
        mut open: impl FnMut(CancellationToken) -> Result<D, String> + Send + 'static,
    ) -> Result<Arc<Self>, AudioError> {
        let state = State::new();
        let worker_state = state.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name(format!("omnivox-pulse-{name}"))
            .spawn(move || {
                let mut initial = true;
                let mut startup = Some(ready_tx);
                loop {
                    let (connection_closed, generation) = {
                        let mut queue = worker_state.queue.lock().unwrap();
                        // Reconnect only for newly admitted audio. Never replay
                        // the failed backlog or spin while the server is down.
                        while !initial
                            && queue.sources.is_empty()
                            && !worker_state.closed.is_cancelled()
                        {
                            queue = worker_state.changed.wait(queue).unwrap();
                        }
                        if worker_state.closed.is_cancelled() {
                            break;
                        }
                        (
                            queue.connection_closed.clone(),
                            worker_state.generation.load(Ordering::Acquire),
                        )
                    };
                    if !initial {
                        tracing::info!(
                            stream = name,
                            "Reopening native PulseAudio output for fresh audio"
                        );
                    }
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            match open(connection_closed.clone()) {
                                Ok(mut device) => {
                                    worker_state.queue.lock().unwrap().connected = true;
                                    if let Some(ready) = startup.take() {
                                        let _ = ready.send(Ok(()));
                                    }
                                    run(&worker_state, &mut device, &connection_closed, generation)
                                }
                                Err(error) => {
                                    if let Some(ready) = startup.take() {
                                        let _ = ready.send(Err(error.clone()));
                                    }
                                    Err(error)
                                }
                            }
                        }))
                        .unwrap_or_else(|_| Err("PulseAudio output worker panicked".into()));
                    if let Some(ready) = startup.take() {
                        // Also unblock construction if native setup panicked.
                        let _ = ready
                            .send(Err(result.clone().err().unwrap_or_else(|| {
                                "PulseAudio output closed during startup".into()
                            })));
                    }
                    if let Err(error) = &result {
                        if !worker_state.shutdown.load(Ordering::Acquire) {
                            tracing::error!(
                                stream = name,
                                %error,
                                "Native PulseAudio output interrupted; fresh audio may reconnect"
                            );
                        }
                    }
                    worker_state.retire(result.err());
                    initial = false;
                }
                worker_state.retire(None);
            })
            .map_err(|error| AudioError::PlaybackError(format!("PulseAudio worker: {error}")))?;
        let sink = Arc::new(Self {
            state,
            worker: Mutex::new(Some(worker)),
        });
        ready_rx
            .recv()
            .map_err(|_| {
                AudioError::DeviceNotFound("PulseAudio worker stopped during startup".into())
            })?
            .map_err(AudioError::DeviceNotFound)?;
        Ok(sink)
    }

    pub(crate) fn lifetime(&self) -> SourceLifetime {
        SourceLifetime {
            generation: self.state.generation.clone(),
            expected: self.state.generation.load(Ordering::Acquire),
            closed: self.state.queue.lock().unwrap().connection_closed.clone(),
        }
    }

    pub(crate) fn append<S: Source<Item = f32> + Send + 'static>(
        &self,
        source: S,
    ) -> Result<(), AudioError> {
        if source.channels() != CHANNELS || source.sample_rate() != SAMPLE_RATE {
            return Err(AudioError::InvalidFormat(
                "PulseAudio requires canonical PCM".into(),
            ));
        }
        let mut queue = self.state.queue.lock().unwrap();
        if self.state.closed.is_cancelled() {
            return Err(AudioError::PlaybackError(
                queue
                    .failure
                    .clone()
                    .unwrap_or_else(|| "PulseAudio output is closed".into()),
            ));
        }
        if queue
            .retry_at
            .is_some_and(|retry_at| Instant::now() < retry_at)
        {
            return Err(AudioError::PlaybackError(
                queue
                    .failure
                    .clone()
                    .unwrap_or_else(|| "PulseAudio reconnect pending".into()),
            ));
        }
        queue.retry_at = None;
        queue.failure = None;
        queue.pending += 1;
        queue.idle = false;
        queue.sources.push_back(Queued {
            source: Box::new(source),
            generation: self.state.generation.load(Ordering::Acquire),
        });
        self.state.changed.notify_all();
        Ok(())
    }

    /// Stream-wide retirement. Selective request cancellation stays in the
    /// shared source, so unrelated requests never cause a downstream flush.
    pub(crate) fn clear(&self) {
        self.state.generation.fetch_add(1, Ordering::AcqRel);
        let sources = {
            let mut queue = self.state.queue.lock().unwrap();
            // A retired worker cannot acknowledge another flush. In
            // particular, stop followed by drain after device failure must
            // not mark the already-closed output busy again.
            if self.state.closed.is_cancelled() || queue.failure.is_some() {
                return;
            }
            let sources = std::mem::take(&mut queue.sources);
            queue.pending -= sources.len();
            queue.idle = !queue.connected;
            sources
        };
        drop(sources);
        self.state.changed.notify_all();
    }

    pub(crate) fn len(&self) -> usize {
        self.state.queue.lock().unwrap().pending
    }

    pub(crate) fn drain(&self) {
        let mut queue = self.state.queue.lock().unwrap();
        while queue.pending != 0 || !queue.idle {
            queue = self.state.changed.wait(queue).unwrap();
        }
    }

    pub(crate) fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::Release);
        self.state.closed.cancel();
        self.state.queue.lock().unwrap().connection_closed.cancel();
        self.state.changed.notify_all();
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

impl Drop for PulseSink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    state: &State,
    device: &mut impl PlaybackDevice,
    connection_closed: &CancellationToken,
    mut generation: u64,
) -> Result<(), String> {
    let mut corked = true;
    let mut primed_frames = 0;
    let mut current: Option<Queued> = None;
    let mut draining = false;
    let mut progress = Instant::now();
    let mut reported = Instant::now();
    let mut samples = Vec::with_capacity(WRITE_FRAMES * CHANNELS as usize);
    loop {
        if state.closed.is_cancelled() || connection_closed.is_cancelled() {
            return if state.shutdown.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err("PulseAudio connection closed".into())
            };
        }
        let observed = state.generation.load(Ordering::Acquire);
        if observed != generation {
            device.cancel_drain();
            draining = false;
            device.active(false);
            device.cork(true)?;
            device.flush()?;
            corked = true;
            primed_frames = 0;
            generation = observed;
            if current
                .as_ref()
                .is_some_and(|source| source.generation != generation)
            {
                state.finish(current.take().unwrap());
            }
            tracing::debug!(
                generation,
                "PulseAudio stream flushed after stop/backlog retirement"
            );
        }
        if current.is_none() {
            current = state.queue.lock().unwrap().sources.pop_front();
            if let Some(source) = &current {
                if source.generation != generation {
                    if source.generation != state.generation.load(Ordering::Acquire) {
                        state.finish(current.take().unwrap());
                    }
                    continue;
                }
                device.cancel_drain();
                draining = false;
                device.active(true);
                progress = Instant::now();
            }
        }
        if current.is_none() {
            // A short source must start even if it cannot fill the startup
            // reserve. Longer sources prime two writes before uncorking, so
            // the first packet does not run dry during the uncork round trip.
            if corked && primed_frames != 0 {
                device.cork(false)?;
                corked = false;
                primed_frames = 0;
            }
            if !corked {
                device.active(false);
                if !draining {
                    device.begin_drain()?;
                    draining = true;
                    progress = Instant::now();
                }
                if device.drained()? {
                    device.cancel_drain();
                    draining = false;
                    device.cork(true)?;
                    corked = true;
                    device.report();
                    tracing::debug!("PulseAudio stream idle and corked");
                } else if progress.elapsed() >= STALL_TIMEOUT {
                    return Err("PulseAudio drain stalled for 3 seconds".into());
                }
            }
            let mut queue = state.queue.lock().unwrap();
            if queue.sources.is_empty() {
                queue.idle = corked;
                state.changed.notify_all();
                let wait = if corked {
                    Duration::from_millis(100)
                } else {
                    POLL
                };
                let _ = state.changed.wait_timeout(queue, wait).unwrap();
            }
            continue;
        }
        let frames = device.writable_frames()?.min(WRITE_FRAMES);
        device.active(true);
        if frames == 0 {
            // The server may negotiate less space than the startup reserve.
            if corked && primed_frames != 0 {
                device.cork(false)?;
                corked = false;
                primed_frames = 0;
                progress = Instant::now();
                continue;
            }
            if progress.elapsed() >= STALL_TIMEOUT {
                return Err("PulseAudio playback stalled for 3 seconds".into());
            }
            std::thread::sleep(POLL);
            continue;
        }
        samples.clear();
        let source = current.as_mut().unwrap();
        for _ in 0..frames * CHANNELS as usize {
            if state.closed.is_cancelled()
                || connection_closed.is_cancelled()
                || state.generation.load(Ordering::Acquire) != generation
            {
                break;
            }
            match source.source.next() {
                Some(sample) => samples.push(sample),
                None => break,
            }
        }
        if state.closed.is_cancelled()
            || connection_closed.is_cancelled()
            || state.generation.load(Ordering::Acquire) != generation
        {
            continue;
        }
        if !samples.len().is_multiple_of(CHANNELS as usize) {
            return Err("PulseAudio source ended inside a PCM frame".into());
        }
        if !samples.is_empty() {
            device.write(&samples)?;
            if corked {
                primed_frames += samples.len() / CHANNELS as usize;
                if primed_frames >= WRITE_FRAMES * 2 {
                    device.cork(false)?;
                    corked = false;
                    primed_frames = 0;
                }
            }
            progress = Instant::now();
            if reported.elapsed() >= Duration::from_secs(1) {
                device.report();
                reported = Instant::now();
            }
        }
        if samples.len() < frames * CHANNELS as usize {
            state.finish(current.take().unwrap());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Fake {
        pub(crate) writable: AtomicBool,
        pub(crate) drained: AtomicBool,
        failed: AtomicBool,
        closed: Mutex<Option<CancellationToken>>,
        writes: Mutex<Vec<Vec<f32>>>,
        events: Mutex<Vec<&'static str>>,
    }

    pub(crate) fn until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "output worker did not make progress"
            );
            std::thread::sleep(POLL);
        }
    }

    impl Fake {
        pub(crate) fn disconnect(&self) {
            self.closed.lock().unwrap().as_ref().unwrap().cancel();
        }
        pub(crate) fn samples(&self) -> Vec<f32> {
            self.writes
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .copied()
                .collect()
        }

        pub(crate) fn events(&self) -> Vec<&'static str> {
            self.events.lock().unwrap().clone()
        }
        fn event(&self, event: &'static str) {
            self.events.lock().unwrap().push(event);
        }
    }

    struct Device(Arc<Fake>);

    impl PlaybackDevice for Device {
        fn writable_frames(&mut self) -> Result<usize, String> {
            if self.0.failed.load(Ordering::Acquire) {
                return Err("injected device failure".into());
            }
            Ok(if self.0.writable.load(Ordering::Acquire) {
                44100
            } else {
                0
            })
        }
        fn write(&mut self, samples: &[f32]) -> Result<(), String> {
            self.0.writes.lock().unwrap().push(samples.to_vec());
            self.0.event("write");
            Ok(())
        }
        fn cork(&mut self, corked: bool) -> Result<(), String> {
            self.0.event(if corked { "cork" } else { "uncork" });
            Ok(())
        }
        fn flush(&mut self) -> Result<(), String> {
            self.0.event("flush");
            Ok(())
        }
        fn active(&mut self, _active: bool) {}
        fn begin_drain(&mut self) -> Result<(), String> {
            self.0.event("drain");
            Ok(())
        }
        fn drained(&mut self) -> Result<bool, String> {
            Ok(self.0.drained.load(Ordering::Acquire))
        }
        fn cancel_drain(&mut self) {
            self.0.event("cancel_drain");
        }
        fn report(&mut self) {}
    }

    pub(crate) fn fake_sink() -> (Arc<PulseSink>, Arc<Fake>) {
        let fake = Arc::new(Fake {
            writable: AtomicBool::new(false),
            drained: AtomicBool::new(true),
            failed: AtomicBool::new(false),
            closed: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let device = fake.clone();
        (
            PulseSink::start("test", move |closed| {
                *device.closed.lock().unwrap() = Some(closed);
                Ok(Device(device.clone()))
            })
            .unwrap(),
            fake,
        )
    }

    fn source(value: f32, frames: usize) -> rodio::buffer::SamplesBuffer<f32> {
        rodio::buffer::SamplesBuffer::new(
            CHANNELS,
            SAMPLE_RATE,
            vec![value; frames * CHANNELS as usize],
        )
    }

    #[test]
    fn latency_request_is_bounded_and_independent_of_alsa_defaults() {
        assert_eq!(latency_ms(None).unwrap(), 20);
        for value in ["10", "20", "50", "200"] {
            assert_eq!(latency_ms(Some(value)).unwrap().to_string(), value);
        }
        for value in ["", "0", "9", "201", "-20", "NaN", "default", "20ms"] {
            assert!(latency_ms(Some(value)).is_err(), "{value}");
        }
    }

    #[test]
    fn bounded_writes_drain_before_idle_cork_and_resume_without_silence() {
        let (sink, fake) = fake_sink();
        fake.drained.store(false, Ordering::Release);
        sink.append(source(0.25, 999)).unwrap();
        fake.writable.store(true, Ordering::Release);
        until(|| fake.events().contains(&"drain"));
        assert!(!fake.events().contains(&"cork"));
        assert_eq!(fake.samples(), vec![0.25; 1998]);
        let events = fake.events();
        let started = events.iter().position(|event| *event == "uncork").unwrap();
        assert_eq!(
            events[..started]
                .iter()
                .filter(|event| **event == "write")
                .count(),
            2
        );
        assert!(fake
            .writes
            .lock()
            .unwrap()
            .iter()
            .all(|write| write.len() <= WRITE_FRAMES * 2));
        fake.drained.store(true, Ordering::Release);
        sink.drain();
        assert!(fake.events().contains(&"cork"));
        sink.append(source(0.5, 1)).unwrap();
        sink.drain();
        assert_eq!(&fake.samples()[1998..], &[0.5, 0.5]);
    }

    #[test]
    fn stop_flushes_old_generation_and_keeps_immediate_replacement() {
        let (sink, fake) = fake_sink();
        sink.append(source(0.25, 999)).unwrap();
        sink.clear();
        sink.append(source(0.5, 5)).unwrap();
        fake.writable.store(true, Ordering::Release);
        sink.drain();
        assert_eq!(fake.samples(), vec![0.5; 10]);
        let events = fake.events();
        assert!(
            events.iter().position(|e| *e == "flush").unwrap()
                < events.iter().position(|e| *e == "write").unwrap()
        );
    }

    #[test]
    fn stop_interrupts_pending_drain_without_waiting_for_the_device_tail() {
        let (sink, fake) = fake_sink();
        fake.drained.store(false, Ordering::Release);
        fake.writable.store(true, Ordering::Release);
        sink.append(source(0.25, 1)).unwrap();
        until(|| fake.events().contains(&"drain"));
        sink.clear();
        sink.drain();
        assert!(fake.events().contains(&"flush"));
    }

    #[test]
    fn device_failure_retires_backlog_and_reconnects_only_for_fresh_audio() {
        let (sink, fake) = fake_sink();
        sink.append(source(0.25, 999)).unwrap();
        sink.append(source(0.5, 999)).unwrap();
        fake.failed.store(true, Ordering::Release);
        until(|| sink.state.queue.lock().unwrap().failure.is_some());
        sink.drain();
        sink.clear();
        assert!(sink.state.queue.lock().unwrap().idle);
        assert!(sink
            .append(source(0.75, 1))
            .unwrap_err()
            .to_string()
            .contains("injected device failure"));
        assert_eq!(sink.len(), 0);
        assert!(fake.samples().is_empty());
        fake.failed.store(false, Ordering::Release);
        fake.writable.store(true, Ordering::Release);
        std::thread::sleep(RECONNECT_DELAY);
        assert!(fake.samples().is_empty());
        sink.append(source(0.75, 1)).unwrap();
        sink.drain();
        assert_eq!(fake.samples(), vec![0.75; 2]);
    }

    #[test]
    fn blocked_lane_does_not_hold_up_another_lane() {
        let (speech, speech_device) = fake_sink();
        let (tone, tone_device) = fake_sink();
        speech.append(source(0.25, 999)).unwrap();
        tone_device.writable.store(true, Ordering::Release);
        tone.append(source(0.5, 8)).unwrap();
        until(|| tone_device.samples().len() == 16);
        assert!(speech_device.samples().is_empty());
        assert_eq!(speech.len(), 1);
        speech.clear();
        speech.drain();
    }

    #[test]
    fn stop_during_reconnect_keeps_only_the_replacement_and_shutdown_stays_final() {
        let (template, fake) = fake_sink();
        template.shutdown();
        let device = fake.clone();
        let (opening_tx, opening_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let mut opens = 0;
        let sink = PulseSink::start("reconnect", move |closed| {
            opens += 1;
            if opens > 1 {
                opening_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
            }
            *device.closed.lock().unwrap() = Some(closed);
            Ok(Device(device.clone()))
        })
        .unwrap();
        let old_lifetime = sink.lifetime();
        fake.disconnect();
        until(|| sink.state.queue.lock().unwrap().failure.is_some());
        assert!(old_lifetime.is_cancelled());
        // Elapse the admission cooldown without slowing a deterministic test.
        sink.state.queue.lock().unwrap().retry_at = None;
        sink.append(source(0.25, 999)).unwrap();
        opening_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        sink.clear();
        sink.drain(); // Opening a connection cannot hold up stop or drain.
        sink.append(source(0.75, 2)).unwrap();
        fake.writable.store(true, Ordering::Release);
        resume_tx.send(()).unwrap();
        sink.drain();
        assert_eq!(fake.samples(), vec![0.75; 4]);
        assert!(old_lifetime.is_cancelled());
        assert!(!sink.lifetime().is_cancelled());
        sink.shutdown();
        assert!(sink.append(source(1.0, 2)).is_err());
        sink.clear();
        sink.drain();
    }

    #[test]
    fn failed_reconnect_does_not_retry_without_fresh_audio() {
        let (template, fake) = fake_sink();
        template.shutdown();
        let device = fake.clone();
        let opens = Arc::new(AtomicU64::new(0));
        let attempts = opens.clone();
        let sink = PulseSink::start("reconnect", move |closed| {
            if attempts.fetch_add(1, Ordering::AcqRel) > 0 {
                return Err("injected reconnect failure".into());
            }
            *device.closed.lock().unwrap() = Some(closed);
            Ok(Device(device.clone()))
        })
        .unwrap();
        fake.disconnect();
        until(|| sink.state.queue.lock().unwrap().failure.is_some());
        assert_eq!(opens.load(Ordering::Acquire), 1);
        sink.state.queue.lock().unwrap().retry_at = None;
        sink.append(source(0.25, 10)).unwrap();
        until(|| {
            sink.state.queue.lock().unwrap().failure.as_deref()
                == Some("injected reconnect failure")
        });
        sink.clear();
        sink.drain();
        assert!(sink.append(source(0.5, 10)).is_err());
        assert_eq!(opens.load(Ordering::Acquire), 2);
        assert!(fake.samples().is_empty());
        sink.shutdown();
        sink.drain();
    }
}
