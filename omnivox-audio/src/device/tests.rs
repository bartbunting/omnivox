use super::*;
use crate::output::{PlaybackStatus, StreamType};
use crate::AudioBuffer;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

type Outputs = Arc<Mutex<Vec<CanonicalQueue>>>;

#[derive(Default)]
struct FakeState {
    device: Mutex<Option<String>>,
    fail: AtomicBool,
    opens: AtomicUsize,
    queries: AtomicUsize,
    outputs: Mutex<Option<Outputs>>,
    signals: Mutex<Option<Arc<Signals>>>,
}

struct FakeBackend {
    state: Arc<FakeState>,
    on_open: Option<Box<dyn FnMut() + Send>>,
}

struct FakeConnection(Outputs);
impl Drop for FakeConnection {
    fn drop(&mut self) {
        self.0.lock().unwrap().clear();
    }
}

impl Backend for FakeBackend {
    type Connection = FakeConnection;
    fn default_device(&mut self) -> Result<Option<String>, AudioError> {
        self.state.queries.fetch_add(1, Ordering::AcqRel);
        Ok(self.state.device.lock().unwrap().clone())
    }
    fn open(&mut self) -> Result<(Self::Connection, [QueueConnection; 3]), AudioError> {
        self.state.opens.fetch_add(1, Ordering::AcqRel);
        if let Some(hook) = &mut self.on_open {
            hook();
        }
        if self.state.fail.load(Ordering::Acquire) {
            return Err(AudioError::DeviceNotFound("injected open failure".into()));
        }
        let mut outputs = Vec::new();
        let queues = std::array::from_fn(|_| {
            let (queue, output) = QueueConnection::new();
            outputs.push(output);
            queue
        });
        let outputs = Arc::new(Mutex::new(outputs));
        *self.state.outputs.lock().unwrap() = Some(outputs.clone());
        Ok((FakeConnection(outputs), queues))
    }
}

fn state() -> Arc<FakeState> {
    Arc::new(FakeState {
        device: Mutex::new(Some("speakers".into())),
        ..FakeState::default()
    })
}

fn owner() -> (Owner<FakeBackend>, Arc<FakeState>) {
    let state = state();
    let (sender, _receiver) = mpsc::sync_channel(EVENT_CAPACITY);
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
    let control = AudioControl::following_device(sinks.clone(), [8, 8, 8]);
    let mut owner = Owner {
        backend: FakeBackend {
            state: state.clone(),
            on_open: None,
        },
        connection: None,
        device: None,
        sinks,
        control,
        signals,
    };
    assert!(owner.reconcile(false).unwrap());
    (owner, state)
}

fn audio(value: f32) -> AudioBuffer {
    AudioBuffer::new(vec![value; 100])
}

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "bounded test wait expired");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn replacement_queues_preserve_stereo_onsets_after_idle() {
    let (mut owner, state) = owner();
    for endpoint in ["speakers", "headphones", "speakers"] {
        *state.device.lock().unwrap() = Some(endpoint.into());
        assert!(owner.reconcile(false).unwrap());
        let outputs = state.outputs.lock().unwrap().clone().unwrap();
        let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 44_100);
        for output in outputs.lock().unwrap().drain(..) {
            controller.add(output);
        }
        for stream in [StreamType::Speech, StreamType::Tone, StreamType::Sound] {
            assert!(mixer.by_ref().take(88_200).all(|sample| sample == 0.0));
            let input: Vec<f32> = (1..=4096).map(|index| index as f32 / 8192.0).collect();
            let ticket = owner
                .control
                .queue_tracked(stream, &AudioBuffer::new(input.clone()))
                .unwrap()
                .unwrap();
            let capture: Vec<_> = mixer.by_ref().take(8192).collect();
            let start = capture
                .iter()
                .position(|value| *value != 0.0)
                .expect("audio onset");
            assert!(start.is_multiple_of(2));
            assert_eq!(&capture[start..start + input.len()], input);
            assert!(capture[start + input.len()..]
                .iter()
                .all(|sample| *sample == 0.0));
            assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        }
        assert!(mixer.by_ref().take(88_200).all(|sample| sample == 0.0));
        let (mut producer, ticket) = owner
            .control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                |_| {},
                CancellationToken::new(),
                || true,
            )
            .unwrap()
            .unwrap();
        let input: Vec<f32> = (1..=4096).map(|index| index as f32 / 8192.0).collect();
        producer
            .push_audio(AudioBuffer::new(input.clone()))
            .unwrap();
        producer.finish().unwrap();
        let capture: Vec<_> = mixer.by_ref().take(8192).collect();
        let start = capture
            .iter()
            .position(|value| *value != 0.0)
            .expect("progressive onset");
        assert_eq!(&capture[start..start + input.len()], input);
        assert!(capture[start + input.len()..]
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }
}

#[test]
fn switching_retires_active_queued_and_overlay_audio_without_replay() {
    let (mut owner, state) = owner();
    let generation = Arc::new(AtomicU64::new(10));
    owner.control.bind_output_generation(generation.clone());
    let first = owner
        .control
        .queue_tracked(StreamType::Speech, &audio(0.1))
        .unwrap()
        .unwrap();
    let second = owner
        .control
        .queue_tracked(StreamType::Speech, &audio(0.2))
        .unwrap()
        .unwrap();
    let tone = owner
        .control
        .queue_tracked(StreamType::Tone, &audio(0.3))
        .unwrap()
        .unwrap();
    let sound = owner
        .control
        .queue_tracked(StreamType::Sound, &audio(0.4))
        .unwrap()
        .unwrap();
    let overlay = owner
        .control
        .queue_overlay_after(&audio(0.5), vec![first.clone()])
        .unwrap()
        .unwrap();
    let outputs = state.outputs.lock().unwrap().clone().unwrap();
    assert_eq!(outputs.lock().unwrap()[0].next(), Some(0.1));
    *state.device.lock().unwrap() = Some("headphones".into());
    assert!(owner.reconcile(false).unwrap());
    for ticket in [first, second, tone, sound, overlay] {
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }
    assert_ne!(generation.load(Ordering::Acquire), 10);
    assert!(!owner
        .control
        .queue_if(StreamType::Speech, &audio(0.6), || generation
            .load(Ordering::Acquire)
            == 10)
        .unwrap());
    let fresh = owner
        .control
        .queue_tracked(StreamType::Speech, &audio(0.7))
        .unwrap()
        .unwrap();
    let outputs = state.outputs.lock().unwrap().clone().unwrap();
    assert!(outputs.lock().unwrap()[0]
        .by_ref()
        .take(100)
        .all(|sample| sample == 0.7));
    outputs.lock().unwrap()[0].next();
    assert_eq!(fresh.wait(), PlaybackStatus::Completed);
}

#[test]
fn switching_unblocks_a_progressive_consumer_and_rejects_old_producer() {
    let (mut owner, state) = owner();
    let (started, waiting) = mpsc::channel();
    let (mut producer, ticket) = owner
        .control
        .queue_progressive_speech_with_cue_callback_cancellable_if(
            move |_| {
                let _ = started.send(());
            },
            CancellationToken::new(),
            || true,
        )
        .unwrap()
        .unwrap();
    producer
        .push_cues(vec![crate::PlaybackCue {
            frame_offset: 0,
            identifier: 1,
        }])
        .unwrap();
    for _ in 0..3 {
        producer.push_audio(audio(0.2)).unwrap();
    }
    let outputs = state.outputs.lock().unwrap().clone().unwrap();
    let source = outputs.lock().unwrap().remove(0);
    let consumer = std::thread::spawn(move || source.collect::<Vec<_>>());
    waiting.recv_timeout(Duration::from_secs(1)).unwrap();
    *state.device.lock().unwrap() = Some("headphones".into());
    assert!(owner.reconcile(false).unwrap());
    consumer.join().unwrap();
    assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    assert!(producer.push_audio(audio(0.8)).is_err());
}

#[test]
fn unprimed_speech_and_unreached_cues_cannot_cross_a_switch() {
    let (mut owner, state) = owner();
    let cues = Arc::new(AtomicUsize::new(0));
    let observed = cues.clone();
    let (mut producer, ticket) = owner
        .control
        .queue_progressive_speech_with_cue_callback_cancellable_if(
            move |_| {
                observed.fetch_add(1, Ordering::AcqRel);
            },
            CancellationToken::new(),
            || true,
        )
        .unwrap()
        .unwrap();
    producer
        .push_cues(vec![crate::PlaybackCue {
            frame_offset: 0,
            identifier: 1,
        }])
        .unwrap();
    producer.push_audio(audio(0.2)).unwrap();
    *state.device.lock().unwrap() = Some("headphones".into());
    owner.reconcile(false).unwrap();
    assert!(producer.finish().is_err());
    assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    assert_eq!(cues.load(Ordering::Acquire), 0);
}

#[test]
fn missing_output_rejects_audio_and_recovery_keeps_only_new_work() {
    let (mut owner, state) = owner();
    let ticket = owner
        .control
        .queue_tracked(StreamType::Sound, &audio(0.1))
        .unwrap()
        .unwrap();
    *state.device.lock().unwrap() = None;
    assert!(owner.reconcile(false).is_err());
    assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    assert!(owner
        .control
        .queue(StreamType::Speech, &audio(0.2))
        .is_err());
    owner.control.stop_all();
    owner.control.drain();
    *state.device.lock().unwrap() = Some("headphones".into());
    assert!(owner.reconcile(false).unwrap());
    assert_eq!(owner.control.pending(StreamType::Speech), 0);
    assert!(owner
        .control
        .queue(StreamType::Speech, &audio(0.3))
        .unwrap());
}

#[test]
fn stale_or_shutdown_connection_attempt_is_never_published() {
    for shutdown in [false, true] {
        let (mut owner, state) = owner();
        let signals = owner.signals.clone();
        owner.backend.on_open = Some(Box::new(move || {
            if shutdown {
                signals.closed.store(true, Ordering::Release);
            } else {
                signals.notify(Event::DefaultChanged);
            }
        }));
        *state.device.lock().unwrap() = Some("headphones".into());
        assert!(!owner.reconcile(false).unwrap());
        assert!(owner.device.is_none());
        assert!(owner.connection.is_none());
        assert!(owner
            .control
            .queue(StreamType::Speech, &audio(0.2))
            .is_err());
    }
}

#[test]
fn duplicate_default_keeps_the_connection() {
    let (mut owner, state) = owner();
    let ticket = owner
        .control
        .queue_tracked(StreamType::Speech, &audio(0.1))
        .unwrap()
        .unwrap();
    owner.signals.notify(Event::DefaultChanged);
    assert!(owner.reconcile(false).unwrap());
    assert_eq!(state.opens.load(Ordering::Acquire), 1);
    assert_eq!(owner.control.pending(StreamType::Speech), 1);
    owner.retire();
    assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
}

#[test]
fn unrelated_endpoint_event_keeps_active_speech() {
    let state = state();
    let native = state.clone();
    let (runtime, control) = DeviceRuntime::start([8, 8, 8], move |signals| {
        *native.signals.lock().unwrap() = Some(signals);
        Ok(FakeBackend {
            state: native,
            on_open: None,
        })
    })
    .unwrap();
    let ticket = control
        .queue_tracked(StreamType::Speech, &audio(0.1))
        .unwrap()
        .unwrap();
    let before = state.queries.load(Ordering::Acquire);
    state
        .signals
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .notify(Event::EndpointChanged("microphone".into()));
    until(|| state.queries.load(Ordering::Acquire) > before);
    assert_eq!(state.opens.load(Ordering::Acquire), 1);
    assert_eq!(control.pending(StreamType::Speech), 1);
    drop(runtime);
    assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
}

#[test]
fn stop_and_shutdown_do_not_allow_a_blocked_attempt_to_publish() {
    let state = state();
    let native = state.clone();
    let (entered, blocked) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (runtime, control) = DeviceRuntime::start([8, 8, 8], move |signals| {
        *native.signals.lock().unwrap() = Some(signals);
        let calls = native.clone();
        Ok(FakeBackend {
            state: native,
            on_open: Some(Box::new(move || {
                if calls.opens.load(Ordering::Acquire) == 2 {
                    entered.send(()).unwrap();
                    resume.recv().unwrap();
                }
            })),
        })
    })
    .unwrap();
    *state.device.lock().unwrap() = Some("headphones".into());
    state
        .signals
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .notify(Event::DefaultChanged);
    blocked.recv_timeout(Duration::from_secs(1)).unwrap();
    let (stopped, observed) = mpsc::channel();
    let stop_control = control.clone();
    let stop = std::thread::spawn(move || {
        stop_control.stop_all();
        stopped.send(()).unwrap();
    });
    observed.recv_timeout(Duration::from_secs(1)).unwrap();
    stop.join().unwrap();
    let signals = runtime.signals.clone();
    let shutdown = std::thread::spawn(move || drop(runtime));
    until(|| signals.closed.load(Ordering::Acquire));
    release.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(control.queue(StreamType::Speech, &audio(0.2)).is_err());
    assert_eq!(state.opens.load(Ordering::Acquire), 2);
}

#[test]
fn retries_are_bounded_and_fresh_audio_can_recover() {
    let state = state();
    let native = state.clone();
    let (runtime, control) = DeviceRuntime::start([8, 8, 8], move |signals| {
        *native.signals.lock().unwrap() = Some(signals);
        Ok(FakeBackend {
            state: native,
            on_open: None,
        })
    })
    .unwrap();
    state.fail.store(true, Ordering::Release);
    state
        .signals
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .notify(Event::EndpointChanged("speakers".into()));
    until(|| state.opens.load(Ordering::Acquire) == 1 + AUTOMATIC_ATTEMPTS);
    std::thread::sleep(RETRY_DELAY * 2);
    assert_eq!(state.opens.load(Ordering::Acquire), 1 + AUTOMATIC_ATTEMPTS);
    state.fail.store(false, Ordering::Release);
    assert!(control.queue(StreamType::Speech, &audio(0.2)).is_err());
    until(|| state.opens.load(Ordering::Acquire) == 2 + AUTOMATIC_ATTEMPTS);
    until(|| {
        control
            .queue(StreamType::Speech, &AudioBuffer::new(vec![0.4; 2]))
            .is_ok()
    });
    drop(runtime);
}

#[test]
fn notification_storm_is_bounded_and_initial_failure_is_an_error() {
    let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
    let signals = Signals {
        sender,
        revision: AtomicU64::new(0),
        overflow: AtomicBool::new(false),
        retry_requested: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    };
    for _ in 0..1000 {
        signals.notify(Event::Rescan);
    }
    assert_eq!(receiver.try_iter().count(), EVENT_CAPACITY);
    assert!(signals.overflow.load(Ordering::Acquire));
    let state = state();
    *state.device.lock().unwrap() = None;
    assert!(DeviceRuntime::start([8, 8, 8], move |_| Ok(FakeBackend {
        state,
        on_open: None
    }))
    .is_err());
}
