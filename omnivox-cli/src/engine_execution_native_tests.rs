// Reuse the ordinary isolation fixtures so both paths have identical blocking behavior.
use super::*;
use omnivox_tts::native_synthesis::{ApplicationStatus, UnavailablePolicy};
use serde_json::json;

fn parameters() -> VoiceParameters {
    serde_json::from_value(json!({
        "native":{"engine_id":"test","schema_id":"test.v1","parameters":{
            "pitch":{"op":"set","value":0},"range":{"op":"default"}}},
        "context_dimensions":["average_pitch"],
        "expected_identity":{"schema_id":"test.v1","profile_id":"test.v1",
            "catalogue_revision":"a".repeat(64),"runtime_generation":1},
        "unavailable_policy":"require"
    }))
    .unwrap()
}

fn receipt(parameters: &VoiceParameters) -> NativeApplication {
    NativeApplication {
        status: ApplicationStatus::Applied,
        plan_id: Some("plan-1".into()),
        identity: Some(parameters.expected_identity.clone()),
        masked_parameters: vec!["pitch".into()],
        reason: None,
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Good,
    Missing,
    Duplicate,
    WrongIdentity,
    Degraded,
    Late,
}
struct NativeEngine {
    blocking: BlockingEngine,
    mode: Mode,
    received: Mutex<Vec<VoiceParameters>>,
}
impl NativeEngine {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            blocking: BlockingEngine::new("test"),
            mode,
            received: Mutex::new(vec![]),
        })
    }
}
impl TtsEngine for NativeEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.blocking.descriptor()
    }
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.blocking.synthesize(request)
    }
    fn synthesize_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &VoiceParameters,
    ) -> Result<(SynthesisResult, NativeApplication), TtsError> {
        self.received.lock().unwrap().push(parameters.clone());
        let result = self.blocking.synthesize(request)?;
        Ok((result, receipt(parameters)))
    }
    fn synthesize_stream_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &VoiceParameters,
        sink: &mut dyn SynthesisStreamSink,
        application: &mut dyn FnMut(&NativeApplication),
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.received.lock().unwrap().push(parameters.clone());
        if matches!(self.mode, Mode::Late) {
            self.blocking.synthesize(request)?;
        }
        let mut applied = receipt(parameters);
        match self.mode {
            Mode::WrongIdentity => applied.identity.as_mut().unwrap().runtime_generation += 1,
            Mode::Degraded => {
                let mut common = parameters.clone();
                common.unavailable_policy = UnavailablePolicy::CommonOnly;
                applied = native_synthesis::common_only(&common)?;
            }
            _ => {}
        }
        if !matches!(self.mode, Mode::Missing) {
            application(&applied);
        }
        if matches!(self.mode, Mode::Duplicate) {
            application(&applied);
        }
        if matches!(self.mode, Mode::Late) {
            sink.start(SynthesisStreamStart {
                engine_id: "test".into(),
                actual_voice: None,
                degraded_acss: vec![],
            })?;
            sink.audio(AudioBuffer::new(vec![0.25, -0.25]))?;
            Ok(SynthesisStreamCompletion { frame_count: 1 })
        } else {
            self.blocking.synthesize_stream(request, sink)
        }
    }
    fn stop(&self) {
        self.blocking.stop();
    }
    fn is_speaking(&self) -> bool {
        self.blocking.is_speaking()
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.blocking.available_voices()
    }
    fn voice_info(&self, id: &str) -> Option<VoiceInfo> {
        self.blocking.voice_info(id)
    }
}
fn wrap(engine: Arc<dyn TtsEngine>, budget: Arc<IsolationBudget>) -> Arc<IsolatedTtsEngine> {
    Arc::new(IsolatedTtsEngine::new(
        engine,
        Arc::new(AtomicU64::new(1)),
        budget,
    ))
}

#[test]
fn unsupported_requires_explicit_degradation_and_preserves_progressive_audio() {
    let engine = Arc::new(BlockingEngine::new("test"));
    let wrapper = wrap(engine.clone(), Arc::new(IsolationBudget::new()));
    let mut parameters = parameters();
    assert!(wrapper
        .synthesize_with_parameters(&request(), &parameters)
        .is_err());
    let (tx, rx) = mpsc::channel();
    let mut sink = SignallingSink { sender: tx.clone() };
    assert!(wrapper
        .synthesize_stream_with_parameters(&request(), &parameters, &mut sink, &mut |_| panic!(
            "strict unsupported request produced a receipt"
        ))
        .is_err());
    parameters.unavailable_policy = UnavailablePolicy::CommonOnly;
    let mut invalid = parameters.clone();
    invalid.expected_identity.catalogue_revision = "invalid".into();
    assert!(wrapper
        .synthesize_with_parameters(&request(), &invalid)
        .is_err());
    assert_eq!(engine.state.lock().unwrap().started, 0);
    let task = thread::spawn(move || {
        wrapper.synthesize_stream_with_parameters(
            &request(),
            &parameters,
            &mut sink,
            &mut |receipt| {
                assert_eq!(receipt.status, ApplicationStatus::CommonOnly);
                tx.send("application").unwrap();
            },
        )
    });
    for expected in ["application", "start", "audio"] {
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), expected);
    }
    assert_eq!(engine.state.lock().unwrap().completed, 0);
    engine.release();
    assert!(task.join().unwrap().is_ok());
}

#[test]
fn native_buffered_and_progressive_requests_keep_values_and_ordinary_speech_clean() {
    let engine = NativeEngine::new(Mode::Good);
    let budget = Arc::new(IsolationBudget::new());
    let wrapper = wrap(engine.clone(), budget.clone());
    let parameters = parameters();
    engine.blocking.release();
    let (_, applied) = wrapper
        .synthesize_with_parameters(&request(), &parameters)
        .unwrap();
    assert_eq!(applied, receipt(&parameters));
    let (tx, rx) = mpsc::channel();
    let task = {
        let wrapper = wrapper.clone();
        let parameters = parameters.clone();
        thread::spawn(move || {
            wrapper.synthesize_stream_with_parameters(
                &request(),
                &parameters,
                &mut SignallingSink { sender: tx.clone() },
                &mut |_| {
                    tx.send("application").unwrap();
                },
            )
        })
    };
    for expected in ["application", "start", "audio"] {
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), expected);
    }
    assert_eq!(engine.blocking.state.lock().unwrap().completed, 1);
    engine.blocking.release();
    assert!(task.join().unwrap().is_ok());
    engine.blocking.release();
    assert!(wrapper.synthesize(&request()).is_ok());
    assert_eq!(
        *engine.received.lock().unwrap(),
        vec![parameters.clone(), parameters]
    );
    wait_for_budget(&budget, 0);
}

#[test]
fn invalid_or_missing_application_never_reaches_the_audio_sink() {
    for mode in [
        Mode::Missing,
        Mode::Duplicate,
        Mode::WrongIdentity,
        Mode::Degraded,
    ] {
        let engine = NativeEngine::new(mode);
        let budget = Arc::new(IsolationBudget::new());
        let wrapper = wrap(engine.clone(), budget.clone());
        engine.blocking.release();
        let (tx, rx) = mpsc::channel();
        assert!(wrapper
            .synthesize_stream_with_parameters(
                &request(),
                &parameters(),
                &mut SignallingSink { sender: tx },
                &mut |_| {}
            )
            .is_err());
        assert!(rx.try_recv().is_err());
        assert!(engine.blocking.stops.load(Ordering::Acquire) > 0);
        wait_for_budget(&budget, 0);
    }
}

#[test]
fn cancelled_native_stream_suppresses_late_application_and_pcm() {
    let engine = NativeEngine::new(Mode::Late);
    let budget = Arc::new(IsolationBudget::new());
    let wrapper = wrap(engine.clone(), budget.clone());
    let token = SynthesisCancellationToken::new();
    let mut request = request();
    request.cancellation = Some(token.clone());
    let (tx, rx) = mpsc::channel();
    let task = thread::spawn(move || {
        wrapper.synthesize_stream_with_parameters(
            &request,
            &parameters(),
            &mut SignallingSink { sender: tx.clone() },
            &mut |_| {
                tx.send("application").unwrap();
            },
        )
    });
    engine.blocking.wait_for_started(1);
    token.cancel();
    engine.blocking.release();
    assert!(task.join().unwrap().is_err());
    assert!(rx.try_recv().is_err());
    wait_for_budget(&budget, 0);
}

#[test]
fn cancelled_native_buffered_result_is_quarantined_and_discarded() {
    let engine = NativeEngine::new(Mode::Good);
    let budget = Arc::new(IsolationBudget::new());
    let wrapper = wrap(engine.clone(), budget.clone());
    let task = {
        let wrapper = wrapper.clone();
        thread::spawn(move || wrapper.synthesize_with_parameters(&request(), &parameters()))
    };
    engine.blocking.wait_for_started(1);
    wrapper.stop();
    assert!(task.join().unwrap().is_err());
    assert_eq!(budget.quarantined(), 1);
    engine.blocking.release();
    wait_for_budget(&budget, 0);
    engine.blocking.release();
    assert!(wrapper
        .synthesize_with_parameters(&request(), &parameters())
        .is_ok());
}
