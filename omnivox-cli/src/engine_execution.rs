//! Generation-aware isolation for native synthesis calls that cannot be preempted.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::{
    AudioBuffer, ResolvedAnchor, SynthesisCancellationToken, SynthesisMarker, SynthesisRequest,
    SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink, SynthesisStreamStart,
    TtsEngine, TtsError, VoiceInfo,
};
use tracing::{info, warn};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SOFT_SUPERSESSION_GRACE: Duration = Duration::from_millis(75);
const TRANSIENT_ENGINE_WAIT: Duration = Duration::from_millis(350);
const ISOLATED_STREAM_EVENT_CAPACITY: usize = 4;
pub const MAX_ISOLATED_CALLS: usize = 2;

enum IsolatedStreamEvent {
    Start(SynthesisStreamStart),
    Audio(AudioBuffer),
    Markers(Vec<SynthesisMarker>, Vec<ResolvedAnchor>),
    Finished(Result<SynthesisStreamCompletion, TtsError>),
}

struct IsolatedStreamRelay {
    sender: mpsc::SyncSender<IsolatedStreamEvent>,
}

impl SynthesisStreamSink for IsolatedStreamRelay {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        self.sender
            .send(IsolatedStreamEvent::Start(start))
            .map_err(|_| TtsError::SynthesisFailed("stream consumer stopped".to_owned()))
    }

    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.sender
            .send(IsolatedStreamEvent::Audio(audio))
            .map_err(|_| TtsError::SynthesisFailed("stream consumer stopped".to_owned()))
    }

    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        self.sender
            .send(IsolatedStreamEvent::Markers(markers, anchors))
            .map_err(|_| TtsError::SynthesisFailed("stream consumer stopped".to_owned()))
    }
}

/// Process-wide admission control for isolated native calls.
///
/// The limit deliberately covers active calls as well as quarantined calls.
/// This is stricter than merely counting abandoned work and guarantees that
/// cancellation can never leave more than two native calls resident.
pub struct IsolationBudget {
    in_flight: AtomicUsize,
    quarantined: AtomicUsize,
}

enum IsolationPressure {
    EngineOccupied,
    ProcessLimit,
}

impl IsolationBudget {
    pub fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            quarantined: AtomicUsize::new(0),
        }
    }

    fn try_acquire(
        self: &Arc<Self>,
        engine_active: &Arc<AtomicBool>,
    ) -> Result<IsolationLease, IsolationPressure> {
        if engine_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(IsolationPressure::EngineOccupied);
        }

        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= MAX_ISOLATED_CALLS {
                engine_active.store(false, Ordering::Release);
                return Err(IsolationPressure::ProcessLimit);
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }

        Ok(IsolationLease(Arc::new(IsolationLeaseInner {
            budget: Arc::clone(self),
            engine_active: Arc::clone(engine_active),
            quarantined: AtomicBool::new(false),
        })))
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn quarantined(&self) -> usize {
        self.quarantined.load(Ordering::Acquire)
    }
}

impl Default for IsolationBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct IsolationLease(Arc<IsolationLeaseInner>);

impl IsolationLease {
    fn mark_quarantined(&self) {
        if !self.0.quarantined.swap(true, Ordering::AcqRel) {
            self.0.budget.quarantined.fetch_add(1, Ordering::AcqRel);
        }
    }
}

struct IsolationLeaseInner {
    budget: Arc<IsolationBudget>,
    engine_active: Arc<AtomicBool>,
    quarantined: AtomicBool,
}

impl Drop for IsolationLeaseInner {
    fn drop(&mut self) {
        if self.quarantined.load(Ordering::Acquire) {
            self.budget.quarantined.fetch_sub(1, Ordering::AcqRel);
        }
        self.budget.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.engine_active.store(false, Ordering::Release);
    }
}

/// Run one engine call away from the sole synthesis worker so a new generation
/// can proceed while an obsolete native call finishes or its helper is killed.
///
/// Only one active-or-quarantined call is permitted for this engine. If that
/// slot, or the process-wide two-call budget, is occupied, synthesis reports
/// the engine unavailable and the ordinary routing layer selects a fallback.
pub struct IsolatedTtsEngine {
    engine: Arc<dyn TtsEngine>,
    generation: Arc<AtomicU64>,
    stop_epoch: AtomicU64,
    engine_active: Arc<AtomicBool>,
    budget: Arc<IsolationBudget>,
    recover_before_next_call: AtomicBool,
}

impl IsolatedTtsEngine {
    pub fn new(
        engine: Arc<dyn TtsEngine>,
        generation: Arc<AtomicU64>,
        budget: Arc<IsolationBudget>,
    ) -> Self {
        Self {
            engine,
            generation,
            stop_epoch: AtomicU64::new(0),
            engine_active: Arc::new(AtomicBool::new(false)),
            budget,
            recover_before_next_call: AtomicBool::new(false),
        }
    }

    fn was_cancelled(
        &self,
        generation: u64,
        stop_epoch: u64,
        cancellation: Option<&SynthesisCancellationToken>,
    ) -> bool {
        self.generation.load(Ordering::Acquire) != generation
            || self.stop_epoch.load(Ordering::Acquire) != stop_epoch
            || cancellation.is_some_and(SynthesisCancellationToken::is_cancelled)
    }

    fn cancellation_error(&self) -> TtsError {
        TtsError::SynthesisFailed(format!(
            "{} synthesis was superseded; its result was discarded",
            self.engine.descriptor().id
        ))
    }

    fn acquire_for_current_generation(
        &self,
        engine_id: &str,
        generation: u64,
        stop_epoch: u64,
        cancellation: Option<&SynthesisCancellationToken>,
    ) -> Result<IsolationLease, TtsError> {
        let deadline = Instant::now() + TRANSIENT_ENGINE_WAIT;
        loop {
            let pressure = match self.budget.try_acquire(&self.engine_active) {
                Ok(lease) => return Ok(lease),
                Err(pressure) => pressure,
            };
            if self.was_cancelled(generation, stop_epoch, cancellation) {
                return Err(self.cancellation_error());
            }
            if Instant::now() >= deadline {
                match pressure {
                    IsolationPressure::EngineOccupied => warn!(
                        engine_id,
                        wait_ms = TRANSIENT_ENGINE_WAIT.as_millis(),
                        "Engine remained occupied after bounded wait; routing through fallback"
                    ),
                    IsolationPressure::ProcessLimit => warn!(
                        engine_id,
                        in_flight = self.budget.in_flight.load(Ordering::Acquire),
                        quarantined = self.budget.quarantined.load(Ordering::Acquire),
                        global_limit = MAX_ISOLATED_CALLS,
                        wait_ms = TRANSIENT_ENGINE_WAIT.as_millis(),
                        "Process-wide isolated synthesis capacity remained occupied; routing through fallback"
                    ),
                }
                return Err(TtsError::NotAvailable);
            }
            thread::sleep(CANCELLATION_POLL_INTERVAL);
        }
    }
}

impl TtsEngine for IsolatedTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.engine.descriptor()
    }

    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        // Connection setup can itself block. Defer it into the same isolated
        // task as synthesis so a newer generation can quarantine it safely.
        self.recover_before_next_call.store(true, Ordering::Release);
        Ok(())
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let engine_id = self.engine.descriptor().id;
        let generation = self.generation.load(Ordering::Acquire);
        let stop_epoch = self.stop_epoch.load(Ordering::Acquire);
        let cancellation = request.cancellation.as_ref();
        let lease =
            self.acquire_for_current_generation(&engine_id, generation, stop_epoch, cancellation)?;
        if self.was_cancelled(generation, stop_epoch, cancellation) {
            return Err(self.cancellation_error());
        }
        let prepare_recovery = self.recover_before_next_call.swap(false, Ordering::AcqRel);
        let engine = Arc::clone(&self.engine);
        let owned_request = request.clone();
        let task_lease = lease.clone();
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let task = thread::Builder::new()
            .name(format!("omnivox-{engine_id}-isolated"))
            .spawn(move || {
                let result = if prepare_recovery {
                    engine
                        .prepare_recovery_probe()
                        .and_then(|()| engine.synthesize(&owned_request))
                } else {
                    engine.synthesize(&owned_request)
                };
                // The caller retains its lease until after it receives this
                // result. Drop the task's clone first so the engine slot is
                // released deterministically when synthesize() returns.
                drop(task_lease);
                let _ = result_sender.send(result);
            });
        if let Err(error) = task {
            if prepare_recovery {
                self.recover_before_next_call.store(true, Ordering::Release);
            }
            return Err(TtsError::SynthesisFailed(format!(
                "could not start isolated {engine_id} synthesis: {error}"
            )));
        }

        let mut superseded_at = None;
        loop {
            match result_receiver.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                Ok(result) => {
                    if self.was_cancelled(generation, stop_epoch, cancellation) {
                        return Err(self.cancellation_error());
                    }
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !self.was_cancelled(generation, stop_epoch, cancellation) {
                        continue;
                    }
                    let hard_stop = self.stop_epoch.load(Ordering::Acquire) != stop_epoch;
                    if !hard_stop {
                        let started = superseded_at.get_or_insert_with(Instant::now);
                        if started.elapsed() < SOFT_SUPERSESSION_GRACE {
                            continue;
                        }
                    }
                    self.engine.stop();
                    lease.mark_quarantined();
                    info!(
                        engine_id,
                        generation,
                        hard_stop,
                        grace_ms = if hard_stop {
                            0
                        } else {
                            SOFT_SUPERSESSION_GRACE.as_millis()
                        },
                        global_limit = MAX_ISOLATED_CALLS,
                        "Quarantined superseded native synthesis"
                    );
                    return Err(self.cancellation_error());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(TtsError::SynthesisFailed(format!(
                        "isolated {engine_id} synthesis task terminated without a result"
                    )));
                }
            }
        }
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        let engine_id = self.engine.descriptor().id;
        let generation = self.generation.load(Ordering::Acquire);
        let stop_epoch = self.stop_epoch.load(Ordering::Acquire);
        let cancellation = request.cancellation.as_ref();
        let lease =
            self.acquire_for_current_generation(&engine_id, generation, stop_epoch, cancellation)?;
        if self.was_cancelled(generation, stop_epoch, cancellation) {
            return Err(self.cancellation_error());
        }
        let prepare_recovery = self.recover_before_next_call.swap(false, Ordering::AcqRel);
        let engine = Arc::clone(&self.engine);
        let owned_request = request.clone();
        let task_lease = lease.clone();
        let (event_sender, event_receiver) = mpsc::sync_channel(ISOLATED_STREAM_EVENT_CAPACITY);
        let task = thread::Builder::new()
            .name(format!("omnivox-{engine_id}-stream"))
            .spawn(move || {
                let mut relay = IsolatedStreamRelay {
                    sender: event_sender.clone(),
                };
                let result = if prepare_recovery {
                    engine
                        .prepare_recovery_probe()
                        .and_then(|()| engine.synthesize_stream(&owned_request, &mut relay))
                } else {
                    engine.synthesize_stream(&owned_request, &mut relay)
                };
                drop(relay);
                drop(task_lease);
                let _ = event_sender.send(IsolatedStreamEvent::Finished(result));
            });
        if let Err(error) = task {
            if prepare_recovery {
                self.recover_before_next_call.store(true, Ordering::Release);
            }
            return Err(TtsError::SynthesisFailed(format!(
                "could not start isolated {engine_id} streaming synthesis: {error}"
            )));
        }

        let mut superseded_at = None;
        loop {
            let cancelled = self.was_cancelled(generation, stop_epoch, cancellation);
            if cancelled {
                let hard_stop = self.stop_epoch.load(Ordering::Acquire) != stop_epoch;
                let grace_expired = hard_stop
                    || superseded_at.get_or_insert_with(Instant::now).elapsed()
                        >= SOFT_SUPERSESSION_GRACE;
                if grace_expired {
                    self.engine.stop();
                    lease.mark_quarantined();
                    info!(
                        engine_id,
                        generation,
                        hard_stop,
                        global_limit = MAX_ISOLATED_CALLS,
                        "Quarantined superseded progressive native synthesis"
                    );
                    return Err(self.cancellation_error());
                }
            }

            match event_receiver.recv_timeout(CANCELLATION_POLL_INTERVAL) {
                Ok(IsolatedStreamEvent::Finished(result)) => {
                    if self.was_cancelled(generation, stop_epoch, cancellation) {
                        return Err(self.cancellation_error());
                    }
                    return result;
                }
                Ok(_) if cancelled => {}
                Ok(IsolatedStreamEvent::Start(start)) => {
                    if let Err(error) = sink.start(start) {
                        self.engine.stop();
                        lease.mark_quarantined();
                        return Err(error);
                    }
                }
                Ok(IsolatedStreamEvent::Audio(audio)) => {
                    if let Err(error) = sink.audio(audio) {
                        self.engine.stop();
                        lease.mark_quarantined();
                        return Err(error);
                    }
                }
                Ok(IsolatedStreamEvent::Markers(markers, anchors)) => {
                    if let Err(error) = sink.markers(markers, anchors) {
                        self.engine.stop();
                        lease.mark_quarantined();
                        return Err(error);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(TtsError::SynthesisFailed(format!(
                        "isolated {engine_id} streaming task terminated without a result"
                    )));
                }
            }
        }
    }

    fn stop(&self) {
        self.stop_epoch.fetch_add(1, Ordering::AcqRel);
        self.engine.stop();
    }

    fn is_speaking(&self) -> bool {
        self.engine_active.load(Ordering::Acquire) || self.engine.is_speaking()
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.engine.available_voices()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.engine.voice_info(identifier)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    use omnivox_tts::contracts::{
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, EngineHealth, MarkerCapabilities,
    };
    use omnivox_tts::{AudioBuffer, TtsSettings, VoiceQuality};

    use super::*;

    struct BlockingState {
        started: usize,
        completed: usize,
        releases: usize,
    }

    struct BlockingEngine {
        id: String,
        state: Mutex<BlockingState>,
        changed: Condvar,
        concurrent: AtomicUsize,
        maximum_concurrent: AtomicUsize,
        stops: AtomicUsize,
    }

    impl BlockingEngine {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_owned(),
                state: Mutex::new(BlockingState {
                    started: 0,
                    completed: 0,
                    releases: 0,
                }),
                changed: Condvar::new(),
                concurrent: AtomicUsize::new(0),
                maximum_concurrent: AtomicUsize::new(0),
                stops: AtomicUsize::new(0),
            }
        }

        fn wait_for_started(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.started < count {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "native synthesis did not start");
                let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.started >= count);
            }
        }

        fn wait_for_completed(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.completed < count {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "native synthesis did not finish");
                let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.completed >= count);
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().unwrap();
            state.releases += 1;
            self.changed.notify_all();
        }
    }

    impl TtsEngine for BlockingEngine {
        fn descriptor(&self) -> EngineDescriptor {
            EngineDescriptor {
                id: self.id.clone(),
                display_name: self.id.clone(),
                version: None,
                availability: Availability::Available,
                health: EngineHealth::Healthy,
                capabilities: EngineCapabilities {
                    acss: AcssCapabilities::default(),
                    audio_output: AudioOutputMode::BufferedPcm,
                    cancellation: CancellationSupport::PlaybackOnly,
                    concurrency: ConcurrencyModel::Serialized,
                    markers: MarkerCapabilities::default(),
                    language_switching: false,
                    text_repertoire: omnivox_tts::contracts::TextRepertoire::Unicode,
                    post_synthesis_dimensions: Vec::new(),
                    native_extensions: Vec::new(),
                },
                voices: Vec::new(),
                default_voice_id: None,
            }
        }

        fn synthesize(&self, _request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            let concurrent = self.concurrent.fetch_add(1, Ordering::AcqRel) + 1;
            self.maximum_concurrent
                .fetch_max(concurrent, Ordering::AcqRel);
            let mut state = self.state.lock().unwrap();
            state.started += 1;
            self.changed.notify_all();
            while state.releases == 0 {
                state = self.changed.wait(state).unwrap();
            }
            state.releases -= 1;
            state.completed += 1;
            self.changed.notify_all();
            self.concurrent.fetch_sub(1, Ordering::AcqRel);
            Ok(SynthesisResult::audio(
                self.id.clone(),
                None,
                AudioBuffer::empty(),
            ))
        }

        fn synthesize_stream(
            &self,
            _request: &SynthesisRequest,
            sink: &mut dyn SynthesisStreamSink,
        ) -> Result<SynthesisStreamCompletion, TtsError> {
            let concurrent = self.concurrent.fetch_add(1, Ordering::AcqRel) + 1;
            self.maximum_concurrent
                .fetch_max(concurrent, Ordering::AcqRel);
            {
                let mut state = self.state.lock().unwrap();
                state.started += 1;
                self.changed.notify_all();
            }
            let emitted = sink
                .start(SynthesisStreamStart {
                    engine_id: self.id.clone(),
                    actual_voice: None,
                    degraded_acss: Vec::new(),
                })
                .and_then(|()| sink.audio(AudioBuffer::new(vec![0.25, -0.25])));
            if let Err(error) = emitted {
                self.concurrent.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
            let mut state = self.state.lock().unwrap();
            while state.releases == 0 {
                state = self.changed.wait(state).unwrap();
            }
            state.releases -= 1;
            state.completed += 1;
            self.changed.notify_all();
            self.concurrent.fetch_sub(1, Ordering::AcqRel);
            Ok(SynthesisStreamCompletion { frame_count: 1 })
        }

        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::AcqRel);
        }

        fn is_speaking(&self) -> bool {
            self.concurrent.load(Ordering::Acquire) != 0
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            vec![VoiceInfo {
                identifier: "default".to_owned(),
                name: "Default".to_owned(),
                language: "en-US".to_owned(),
                quality: VoiceQuality::Compact,
            }]
        }

        fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
            self.available_voices()
                .into_iter()
                .find(|voice| voice.identifier == identifier)
        }
    }

    fn request() -> SynthesisRequest {
        SynthesisRequest::new("blocking", TtsSettings::default())
    }

    fn isolated(
        engine: Arc<BlockingEngine>,
        generation: Arc<AtomicU64>,
        budget: Arc<IsolationBudget>,
    ) -> Arc<IsolatedTtsEngine> {
        let erased: Arc<dyn TtsEngine> = engine;
        Arc::new(IsolatedTtsEngine::new(erased, generation, budget))
    }

    fn wait_for_budget(budget: &IsolationBudget, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while budget.in_flight() != expected {
            assert!(Instant::now() < deadline, "isolation slot was not released");
            thread::yield_now();
        }
    }

    struct SignallingSink {
        sender: mpsc::Sender<&'static str>,
    }

    impl SynthesisStreamSink for SignallingSink {
        fn start(&mut self, _start: SynthesisStreamStart) -> Result<(), TtsError> {
            self.sender.send("start").unwrap();
            Ok(())
        }

        fn audio(&mut self, _audio: AudioBuffer) -> Result<(), TtsError> {
            self.sender.send("audio").unwrap();
            Ok(())
        }

        fn markers(
            &mut self,
            _markers: Vec<SynthesisMarker>,
            _anchors: Vec<ResolvedAnchor>,
        ) -> Result<(), TtsError> {
            self.sender.send("markers").unwrap();
            Ok(())
        }
    }

    #[test]
    fn progressive_windows_cross_isolation_before_native_completion() {
        let generation = Arc::new(AtomicU64::new(1));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("progressive"));
        let engine = isolated(Arc::clone(&native), generation, Arc::clone(&budget));
        let (event_tx, event_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let mut sink = SignallingSink { sender: event_tx };
            let _ = finished_tx.send(caller.synthesize_stream(&request(), &mut sink));
        });

        native.wait_for_started(1);
        assert_eq!(
            event_rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            "start"
        );
        assert_eq!(
            event_rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            "audio"
        );
        assert!(matches!(
            finished_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        native.release();
        assert_eq!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap()
                .frame_count,
            1
        );
        wait_for_budget(&budget, 0);
    }

    #[test]
    fn cancelling_a_progressive_call_releases_its_bounded_relay() {
        let generation = Arc::new(AtomicU64::new(1));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("progressive-cancel"));
        let engine = isolated(Arc::clone(&native), generation, Arc::clone(&budget));
        let cancellation = SynthesisCancellationToken::new();
        let cancellable = request().with_cancellation(cancellation.clone());
        let (event_tx, event_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let mut sink = SignallingSink { sender: event_tx };
            let _ = finished_tx.send(caller.synthesize_stream(&cancellable, &mut sink));
        });

        native.wait_for_started(1);
        assert_eq!(
            event_rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            "start"
        );
        assert_eq!(
            event_rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            "audio"
        );
        cancellation.cancel();
        assert!(matches!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        assert_eq!(native.stops.load(Ordering::Acquire), 1);
        assert_eq!(budget.quarantined(), 1);

        native.release();
        native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
    }

    #[test]
    fn soft_supersession_discards_a_natural_completion_without_stopping_engine() {
        let generation = Arc::new(AtomicU64::new(5));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("soft"));
        let engine = isolated(
            Arc::clone(&native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = finished_tx.send(caller.synthesize(&request()));
        });

        native.wait_for_started(1);
        generation.store(6, Ordering::Release);
        native.release();
        assert!(matches!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
        assert_eq!(budget.quarantined(), 0);
        assert_eq!(native.stops.load(Ordering::Acquire), 0);

        native.release();
        assert!(engine.synthesize(&request()).is_ok());
    }

    #[test]
    fn request_token_cancels_one_isolated_call_without_advancing_global_generation() {
        let generation = Arc::new(AtomicU64::new(9));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("keyed"));
        let engine = isolated(
            Arc::clone(&native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let cancellation = SynthesisCancellationToken::new();
        let cancellable = request().with_cancellation(cancellation.clone());
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = finished_tx.send(caller.synthesize(&cancellable));
        });

        native.wait_for_started(1);
        cancellation.cancel();
        assert!(matches!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        assert_eq!(generation.load(Ordering::Acquire), 9);
        assert_eq!(native.stops.load(Ordering::Acquire), 1);
        assert_eq!(budget.quarantined(), 1);

        native.release();
        native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
        native.release();
        assert!(engine.synthesize(&request()).is_ok());
    }

    #[test]
    fn current_generation_waits_for_a_transient_engine_occupant() {
        let generation = Arc::new(AtomicU64::new(11));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("transient"));
        let engine = isolated(
            Arc::clone(&native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );

        let (old_tx, old_rx) = mpsc::channel();
        let old_caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = old_tx.send(old_caller.synthesize(&request()));
        });
        native.wait_for_started(1);

        generation.store(12, Ordering::Release);
        let (current_tx, current_rx) = mpsc::channel();
        let current_caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = current_tx.send(current_caller.synthesize(&request()));
        });
        thread::sleep(Duration::from_millis(20));
        assert!(matches!(
            current_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        native.release();
        assert!(matches!(
            old_rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        native.wait_for_started(2);
        native.release();
        assert!(current_rx
            .recv_timeout(Duration::from_millis(250))
            .unwrap()
            .is_ok());
        wait_for_budget(&budget, 0);
        assert_eq!(native.maximum_concurrent.load(Ordering::Acquire), 1);
    }

    #[test]
    fn newer_generation_returns_without_waiting_and_stale_result_is_discarded() {
        let generation = Arc::new(AtomicU64::new(7));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("blocked"));
        let engine = isolated(
            Arc::clone(&native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = finished_tx.send(caller.synthesize(&request()));
        });

        native.wait_for_started(1);
        generation.store(8, Ordering::Release);
        let cancelled = finished_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("isolated caller remained blocked");
        assert!(matches!(cancelled, Err(TtsError::SynthesisFailed(_))));
        assert_eq!(budget.in_flight(), 1);
        assert_eq!(budget.quarantined(), 1);

        let fallback_signal = engine.synthesize(&request());
        assert!(matches!(fallback_signal, Err(TtsError::NotAvailable)));
        assert_eq!(native.state.lock().unwrap().started, 1);

        native.release();
        native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
        assert_eq!(budget.in_flight(), 0);
        assert_eq!(budget.quarantined(), 0);

        native.release();
        assert!(engine.synthesize(&request()).is_ok());
        assert_eq!(native.maximum_concurrent.load(Ordering::Acquire), 1);
    }

    #[test]
    fn repeated_stop_never_creates_a_second_abandoned_call_for_an_engine() {
        let generation = Arc::new(AtomicU64::new(1));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("serial"));
        let engine = isolated(Arc::clone(&native), generation, Arc::clone(&budget));
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let _ = finished_tx.send(caller.synthesize(&request()));
        });

        native.wait_for_started(1);
        engine.stop();
        engine.stop();
        assert!(matches!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        assert!(matches!(
            engine.synthesize(&request()),
            Err(TtsError::NotAvailable)
        ));
        assert_eq!(native.state.lock().unwrap().started, 1);
        assert_eq!(budget.quarantined(), 1);
        assert_eq!(native.stops.load(Ordering::Acquire), 3);

        native.release();
        native.wait_for_completed(1);
    }

    #[test]
    fn global_budget_rejects_a_third_native_call() {
        let generation = Arc::new(AtomicU64::new(3));
        let budget = Arc::new(IsolationBudget::new());
        let first_native = Arc::new(BlockingEngine::new("first"));
        let second_native = Arc::new(BlockingEngine::new("second"));
        let third_native = Arc::new(BlockingEngine::new("third"));
        let first = isolated(
            Arc::clone(&first_native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let second = isolated(
            Arc::clone(&second_native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let third = isolated(
            Arc::clone(&third_native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );

        let (finished_tx, finished_rx) = mpsc::channel();
        for engine in [first, second] {
            let finished_tx = finished_tx.clone();
            thread::spawn(move || {
                let _ = finished_tx.send(engine.synthesize(&request()));
            });
        }
        first_native.wait_for_started(1);
        second_native.wait_for_started(1);
        assert_eq!(budget.in_flight(), MAX_ISOLATED_CALLS);
        assert!(matches!(
            third.synthesize(&request()),
            Err(TtsError::NotAvailable)
        ));
        assert_eq!(third_native.state.lock().unwrap().started, 0);

        generation.store(4, Ordering::Release);
        for _ in 0..2 {
            assert!(matches!(
                finished_rx
                    .recv_timeout(Duration::from_millis(250))
                    .unwrap(),
                Err(TtsError::SynthesisFailed(_))
            ));
        }
        first_native.release();
        second_native.release();
        first_native.wait_for_completed(1);
        second_native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
        assert_eq!(budget.in_flight(), 0);
    }

    #[test]
    fn dropping_the_wrapper_does_not_join_a_quarantined_native_call() {
        let generation = Arc::new(AtomicU64::new(11));
        let budget = Arc::new(IsolationBudget::new());
        let native = Arc::new(BlockingEngine::new("shutdown"));
        let engine = isolated(
            Arc::clone(&native),
            Arc::clone(&generation),
            Arc::clone(&budget),
        );
        let weak = Arc::downgrade(&engine);
        let (finished_tx, finished_rx) = mpsc::channel();
        let caller = Arc::clone(&engine);
        thread::spawn(move || {
            let result = caller.synthesize(&request());
            drop(caller);
            let _ = finished_tx.send(result);
        });

        native.wait_for_started(1);
        generation.store(12, Ordering::Release);
        assert!(matches!(
            finished_rx
                .recv_timeout(Duration::from_millis(250))
                .unwrap(),
            Err(TtsError::SynthesisFailed(_))
        ));
        let started_at = Instant::now();
        drop(engine);
        assert!(started_at.elapsed() < Duration::from_millis(100));
        assert!(weak.upgrade().is_none());
        assert_eq!(budget.quarantined(), 1);

        native.release();
        native.wait_for_completed(1);
        wait_for_budget(&budget, 0);
    }
}
