use super::*;
use omnivox_tts::engine_voice_choices::*;
use omnivox_tts::native_parameters::{CommonInput, NativeValue, ParameterCatalogue};
use omnivox_tts::native_synthesis::*;
use serde_json::json;

#[derive(Clone, Copy, Default)]
enum ReceiptBehavior {
    #[default]
    Good,
    Missing,
    Duplicate,
    Stale,
    CommonOnly,
    Late,
    DuplicateAfterAudio,
    Cancel,
    Empty,
    EmptyWhitespace,
    EmptyWithoutStart,
    WrongCount,
}
struct NativeEngine {
    inner: Arc<MockEngine>,
    behavior: ReceiptBehavior,
    native_calls: Mutex<Vec<(SynthesisRequest, VoiceParameters)>>,
    supersede: Option<Arc<AtomicU64>>,
    epoch: AtomicU64,
}
impl NativeEngine {
    fn new(
        id: &str,
        voice: &str,
        streaming: bool,
        failure: Option<MockFailure>,
        behavior: ReceiptBehavior,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: full_engine(id, voice, streaming, failure),
            behavior,
            native_calls: Mutex::new(vec![]),
            supersede: None,
            epoch: AtomicU64::new(1),
        })
    }
    fn receipt(&self, p: &VoiceParameters) -> NativeApplication {
        let mut receipt = NativeApplication {
            status: ApplicationStatus::Applied,
            plan_id: Some(format!("{}-plan", self.inner.descriptor.id)),
            identity: Some(p.expected_identity.clone()),
            masked_parameters: vec![],
            reason: None,
        };
        if matches!(self.behavior, ReceiptBehavior::Stale) {
            receipt.identity.as_mut().unwrap().runtime_generation += 1;
        }
        if matches!(self.behavior, ReceiptBehavior::CommonOnly) {
            receipt = NativeApplication {
                status: ApplicationStatus::CommonOnly,
                plan_id: None,
                identity: None,
                masked_parameters: vec![],
                reason: Some("runtime changed".into()),
            };
        }
        receipt
    }
    fn cancel(&self, request: &SynthesisRequest) {
        if let Some(generation) = &self.supersede {
            generation.store(2, Ordering::Release);
        }
        if matches!(self.behavior, ReceiptBehavior::Cancel) {
            request.cancellation.as_ref().unwrap().cancel();
        }
    }
}
impl TtsEngine for NativeEngine {
    fn parameter_cache_epoch(&self) -> Option<u64> { Some(self.epoch.load(Ordering::Acquire)) }
    fn descriptor(&self) -> EngineDescriptor {
        self.inner.descriptor()
    }
    fn synthesize(&self, r: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let mut result = self.inner.synthesize(r)?;
        result.audio = AudioBuffer::new(vec![0.2, -0.2]);
        Ok(result)
    }
    fn synthesize_stream(
        &self,
        r: &SynthesisRequest,
        s: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.inner.synthesize_stream(r, s)
    }
    fn synthesize_with_parameters(
        &self,
        request: &SynthesisRequest,
        p: &VoiceParameters,
    ) -> Result<(SynthesisResult, NativeApplication), TtsError> {
        self.native_calls
            .lock()
            .unwrap()
            .push((request.clone(), p.clone()));
        let mut result = self.inner.synthesize(request)?;
        result.audio = AudioBuffer::new(vec![0.2, -0.2]);
        self.cancel(request);
        Ok((result, self.receipt(p)))
    }
    fn synthesize_stream_with_parameters(
        &self,
        r: &SynthesisRequest,
        p: &VoiceParameters,
        sink: &mut dyn SynthesisStreamSink,
        application: &mut dyn FnMut(&NativeApplication),
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.native_calls
            .lock()
            .unwrap()
            .push((r.clone(), p.clone()));
        let receipt = self.receipt(p);
        if !matches!(
            self.behavior,
            ReceiptBehavior::Missing | ReceiptBehavior::Late
        ) {
            application(&receipt);
        }
        if matches!(self.behavior, ReceiptBehavior::Duplicate) {
            application(&receipt);
        }
        self.cancel(r);
        if matches!(self.behavior, ReceiptBehavior::EmptyWithoutStart) {
            sink.audio(AudioBuffer::empty())?;
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }
        if matches!(self.behavior, ReceiptBehavior::Empty)
            || (matches!(self.behavior, ReceiptBehavior::EmptyWhitespace)
                && r.text.trim().is_empty())
        {
            sink.start(SynthesisStreamStart {
                engine_id: self.descriptor().id,
                actual_voice: r.requested_voice.clone(),
                degraded_acss: vec![],
            })?;
            sink.audio(AudioBuffer::empty())?;
            return Ok(SynthesisStreamCompletion { frame_count: 0 });
        }
        let mut result = self.inner.synthesize_stream(r, sink);
        if matches!(
            self.behavior,
            ReceiptBehavior::Late | ReceiptBehavior::DuplicateAfterAudio
        ) {
            application(&receipt);
        }
        if matches!(self.behavior, ReceiptBehavior::WrongCount) {
            if let Ok(c) = &mut result {
                c.frame_count += 1;
            }
        }
        result
    }
    fn stop(&self) {}
    fn is_speaking(&self) -> bool {
        false
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        vec![]
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        None
    }
}
fn metadata(id: &str, voice: &str) -> ParameterCatalogue {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    let mut value = fixtures["messages"]["catalogue_response"]["result"].clone();
    let object = value.as_object_mut().unwrap();
    object.remove("status");
    object.remove("next_cursor");
    object.insert("engine_id".into(), json!(id));
    object.insert("voice_id".into(), json!(voice));
    ParameterCatalogue::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}
fn native_definition() -> EngineLayeredVoiceDefinition {
    let common = fixture_voice();
    EngineLayeredVoiceDefinition { id:common.id, language:common.language, shared:common.shared,
        choices:common.choices.into_iter().enumerate().map(|(i,c)| EngineVoiceChoice {
            native: Some(serde_json::from_value(json!({"engine_id":c.selector.engine_id(),"schema_id":metadata("dectalk","Paul").identity.schema_id,
                "parameters":{"sm":{"op":"set","value":([0,80,20][i])},"ri":{"op":"default"}}})).unwrap()),
            id:c.id, selector:c.selector, adjustments:c.adjustments,
        }).collect() }
}
fn snapshot(
    engines: &EngineRegistry,
    definition: EngineLayeredVoiceDefinition,
) -> LogicalVoiceRoutingSnapshot {
    let inventory = engines.inventory();
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v3(
            VoiceRegistrationV3 {
                registry_generation: 41,
                definitions: vec![EngineRegisteredVoiceDefinition::EngineLayered(definition)],
                fallback_policy: omnivox_tts::control::ChoiceFallbackPolicy {
                    preferred_engines: vec![],
                    allow_same_language_on_requested_engine: false,
                    global_default: None,
                    fallback_engines: vec![],
                },
            },
            &NativeCatalogueSnapshot::new(&inventory, &[]).unwrap(),
        )
        .unwrap();
    LogicalVoiceRoutingSnapshot::capture(&registry, engines)
}
fn style<'a>(
    context: &'a VoiceStylePatch,
    knowledge: &'a [ParameterKnowledge<'a>],
    policy: UnavailablePolicy,
) -> AttemptStyle<'a> {
    AttemptStyle::EngineLayered {
        context,
        base_rate: 0.5,
        placement_pan: Some(0.3),
        knowledge,
        policy,
    }
}
struct NativeSink {
    inner: AttemptSink,
    supported: bool,
}
impl Default for NativeSink {
    fn default() -> Self {
        Self {
            inner: AttemptSink::default(),
            supported: true,
        }
    }
}
impl RoutedPlaybackSink for NativeSink {
    fn supports_native(&self) -> bool {
        self.supported
    }
    fn preflight_attempt(&mut self, attempt: &PreparedVoiceAttempt) -> Result<(), TtsError> {
        assert!(attempt.native_application.is_none());
        self.inner.preflight_attempt(attempt)
    }
    fn start_attempt(
        &mut self,
        attempt: &PreparedVoiceAttempt,
        start: SynthesisStreamStart,
    ) -> Result<(), TtsError> {
        self.inner.start_attempt(attempt, start)
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.inner.audio(audio)
    }
    fn markers(&mut self, m: Vec<SynthesisMarker>, a: Vec<ResolvedAnchor>) -> Result<(), TtsError> {
        self.inner.markers(m, a)
    }
}
fn execute(
    engines: &EngineRegistry,
    routing: &mut LogicalVoiceRoutingSnapshot,
    style: &AttemptStyle<'_>,
    sink: &mut NativeSink,
    cancellation: Option<&SynthesisCancellationToken>,
) -> PreparedSynthesisOutcome {
    let mut route = routing.initial_native_route("bolden", engines).unwrap();
    synthesize_prepared_with_runtime_fallback_anchored(
        "the quick brown fox",
        &[],
        style,
        &mut route,
        routing,
        engines,
        &RuntimeEngineHealth::default(),
        1,
        &AtomicU64::new(1),
        cancellation,
        sink,
    )
}
fn actual(outcome: PreparedSynthesisOutcome, sink: &NativeSink) -> PreparedVoiceAttempt {
    match outcome {
        PreparedSynthesisOutcome::Buffered { attempt, .. } => *attempt,
        PreparedSynthesisOutcome::Streamed(_) => sink.inner.attempts.last().unwrap().clone(),
        _ => panic!("expected synthesis success"),
    }
}
#[test]
fn native_fallback_recomposes_all_buffered_and_streaming_combinations() {
    let catalogues = [metadata("dectalk", "Paul"), metadata("eloquence", "Reed")];
    let knowledge = catalogues
        .iter()
        .map(ParameterKnowledge::Ready)
        .collect::<Vec<_>>();
    let context = VoiceStylePatch {
        richness: Some(Adjustment::Set { value: 0.5 }),
        ..Default::default()
    };
    for first in [false, true] {
        for second in [false, true] {
            let primary = NativeEngine::new(
                "dectalk",
                "Paul",
                first,
                Some(if first {
                    MockFailure::StreamAfterMarkers
                } else {
                    MockFailure::Synthesis
                }),
                ReceiptBehavior::Good,
            );
            let fallback =
                NativeEngine::new("eloquence", "Reed", second, None, ReceiptBehavior::Good);
            let mut engines = EngineRegistry::new();
            engines.register(primary.clone()).unwrap();
            engines.register(fallback.clone()).unwrap();
            let mut routing = snapshot(&engines, native_definition());
            let mut sink = NativeSink::default();
            let result = execute(
                &engines,
                &mut routing,
                &style(&context, &knowledge, UnavailablePolicy::Require),
                &mut sink,
                None,
            );
            let attempt = actual(result, &sink);
            assert_eq!(attempt.choice_id.as_deref(), Some("eloquence-reed"));
            assert_eq!(
                attempt
                    .native_application
                    .as_ref()
                    .unwrap()
                    .plan_id
                    .as_deref(),
                Some("eloquence-plan")
            );
            assert!(sink
                .inner
                .attempts
                .iter()
                .all(|a| a.resolution.realized.engine_id == "eloquence"));
            assert_eq!(attempt.acss.style.richness, Some(0.5));
            assert_eq!(attempt.acss.style.average_pitch, Some(0.6));
            assert_eq!(attempt.effects.style.low_pass, Some(0.33));
            assert_eq!(attempt.effects.style.pan, Some(0.3));
            let a = primary.native_calls.lock().unwrap();
            let b = fallback.native_calls.lock().unwrap();
            assert_eq!(a[0].0.text, b[0].0.text);
            assert_eq!(
                a[0].1.native.parameters["sm"],
                Adjustment::Set {
                    value: NativeValue::Integer(0)
                }
            );
            assert_eq!(
                b[0].1.native.parameters["sm"],
                Adjustment::Set {
                    value: NativeValue::Integer(80)
                }
            );
            assert_eq!(b[0].1.native.parameters["ri"], Adjustment::Default {});
            assert_eq!(b[0].1.context_dimensions, vec![CommonInput::Richness]);
        }
    }
}
#[test]
fn exact_private_native_preview_keeps_same_voice_choice_occurrence_and_has_no_substitute() {
    let engine = NativeEngine::new("eloquence", "Reed", true, None, ReceiptBehavior::Good);
    let mut engines = EngineRegistry::new();
    engines.register(engine.clone()).unwrap();
    let catalogue = metadata("eloquence", "Reed");
    let knowledge = [ParameterKnowledge::Ready(&catalogue)];
    for index in [1, 2] {
        let mut routing = snapshot(&engines, native_definition());
        routing.restrict_preview_to_choice(index).unwrap();
        let mut sink = NativeSink::default();
        let attempt = actual(
            execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
                &mut sink,
                None,
            ),
            &sink,
        );
        assert_eq!(attempt.choice_index, Some(index));
        let NativeChoiceExecution::Parameters(p) = attempt.native else {
            panic!()
        };
        assert_eq!(
            p.native.parameters["sm"],
            Adjustment::Set {
                value: NativeValue::Integer([0, 80, 20][index])
            }
        );
    }
    let primary = NativeEngine::new(
        "dectalk",
        "Paul",
        true,
        Some(MockFailure::StreamBeforeAudio),
        ReceiptBehavior::Good,
    );
    engines.register(primary).unwrap();
    let mut routing = snapshot(&engines, native_definition());
    routing.restrict_preview_to_choice(0).unwrap();
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    let calls = engine.native_calls.lock().unwrap().len();
    assert!(matches!(
        execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut NativeSink::default(),
            None
        ),
        PreparedSynthesisOutcome::Exhausted
    ));
    assert_eq!(engine.native_calls.lock().unwrap().len(), calls);
}
#[test]
fn missing_metadata_requires_explicit_whole_block_degradation() {
    for streaming in [false, true] {
        let engine = NativeEngine::new("dectalk", "Paul", streaming, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut sink = NativeSink::default();
        assert!(matches!(
            execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &[], UnavailablePolicy::Require),
                &mut sink,
                None
            ),
            PreparedSynthesisOutcome::Failed
        ));
        assert!(engine.inner.calls.lock().unwrap().is_empty());
        let result = execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &[], UnavailablePolicy::CommonOnly),
            &mut sink,
            None,
        );
        let a = actual(result, &sink);
        let receipt = a.native_application.unwrap();
        assert_eq!(receipt.status, ApplicationStatus::CommonOnly);
        assert!(receipt.plan_id.is_none());
        assert!(engine.native_calls.lock().unwrap().is_empty());
        assert_eq!(a.acss.style.average_pitch, Some(0.2));
    }
}
#[test]
fn malformed_receipts_are_quarantined_before_audio_and_do_not_leak_into_fallback() {
    let catalogues = [metadata("dectalk", "Paul"), metadata("eloquence", "Reed")];
    let knowledge = catalogues
        .iter()
        .map(ParameterKnowledge::Ready)
        .collect::<Vec<_>>();
    for behavior in [
        ReceiptBehavior::Missing,
        ReceiptBehavior::Duplicate,
        ReceiptBehavior::Stale,
        ReceiptBehavior::CommonOnly,
        ReceiptBehavior::Late,
    ] {
        let primary = NativeEngine::new("dectalk", "Paul", true, None, behavior);
        let fallback = NativeEngine::new("eloquence", "Reed", true, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(fallback).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut sink = NativeSink::default();
        let a = actual(
            execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
                &mut sink,
                None,
            ),
            &sink,
        );
        assert_eq!(a.resolution.realized.engine_id, "eloquence");
        assert_eq!(sink.inner.attempts.len(), 1);
        assert_eq!(
            a.native_application.unwrap().plan_id.as_deref(),
            Some("eloquence-plan")
        );
    }
}
#[test]
fn accepted_audio_prevents_replay_on_synthesis_receipt_or_completion_failure() {
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    for (failure, behavior) in [
        (Some(MockFailure::StreamAfterAudio), ReceiptBehavior::Good),
        (None, ReceiptBehavior::DuplicateAfterAudio),
        (None, ReceiptBehavior::WrongCount),
    ] {
        let primary = NativeEngine::new("dectalk", "Paul", true, failure, behavior);
        let fallback = NativeEngine::new("eloquence", "Reed", true, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(fallback.clone()).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut sink = NativeSink::default();
        assert!(matches!(
            execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
                &mut sink,
                None
            ),
            PreparedSynthesisOutcome::Failed
        ));
        assert_eq!(sink.inner.attempts.len(), 1);
        assert!(fallback.inner.calls.lock().unwrap().is_empty());
    }
}
#[test]
fn cancellation_suppresses_tentative_native_evidence_and_buffered_results() {
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    for streaming in [false, true] {
        let primary =
            NativeEngine::new("dectalk", "Paul", streaming, None, ReceiptBehavior::Cancel);
        let fallback = NativeEngine::new("eloquence", "Reed", true, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(fallback.clone()).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut sink = NativeSink::default();
        let cancellation = SynthesisCancellationToken::default();
        assert!(matches!(
            execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
                &mut sink,
                Some(&cancellation)
            ),
            PreparedSynthesisOutcome::Cancelled
        ));
        assert!(sink.inner.attempts.is_empty());
        assert!(fallback.inner.calls.lock().unwrap().is_empty());
    }
}
#[test]
fn empty_native_stream_does_not_commit_application_evidence() {
    let engine = NativeEngine::new("dectalk", "Paul", true, None, ReceiptBehavior::Empty);
    let mut engines = EngineRegistry::new();
    engines.register(engine).unwrap();
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    let mut routing = snapshot(&engines, native_definition());
    let mut sink = NativeSink::default();
    let outcome = execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut sink,
            None
        );
    let PreparedSynthesisOutcome::Buffered { result, attempt } = outcome else {
        panic!("empty native output must remain available for pipeline completion");
    };
    assert!(result.audio.is_empty());
    assert_eq!(result.actual_voice, Some(PhysicalVoiceId::new("dectalk", "Paul")));
    assert!(attempt.native_application.is_some());
    assert!(sink.inner.attempts.is_empty());
    let broken = NativeEngine::new(
        "dectalk",
        "Paul",
        true,
        None,
        ReceiptBehavior::EmptyWithoutStart,
    );
    let mut engines = EngineRegistry::new();
    engines.register(broken).unwrap();
    let mut routing = snapshot(&engines, native_definition());
    let mut sink = NativeSink::default();
    assert!(matches!(
        execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut sink,
            None
        ),
        PreparedSynthesisOutcome::Exhausted
    ));
    assert!(sink.inner.attempts.is_empty());
}

#[test]
fn native_timeline_continues_after_silent_markdown_punctuation() {
    use crate::pipeline::{process_presentation_timeline_v4, BatchStatus, DispatchEffects, SynthCtx};
    use omnivox_audio::{AudioBackend, AudioFileLoader, AudioStreams, PlaybackStatus, TimelineAudioRenderer};
    use omnivox_tts::timeline_protocol::PresentationDeliveryPolicy;
    use omnivox_tts::timeline_v4::{LayeredSpeechSpan, MixedSpeechSpan, PresentationTimelineV4};

    let engine = NativeEngine::new("dectalk", "Paul", true, None, ReceiptBehavior::EmptyWhitespace);
    let mut engines = EngineRegistry::new();
    engines.register(engine.clone()).unwrap();
    let routing = snapshot(&engines, native_definition())
        .with_parameter_catalogues(vec![Arc::new(metadata("dectalk", "Paul"))]);
    let streams = AudioStreams::new_with_backend(8, 8, 8, AudioBackend::Null).unwrap();
    let control = streams.control();
    let generation = AtomicU64::new(1);
    let lifecycle = crate::lifecycle::RequestLifecycle::default();
    let tickets = Mutex::new(Vec::new());
    let effects = Mutex::new(DispatchEffects::new());
    let renderer = Mutex::new(TimelineAudioRenderer::new());
    let observations = Mutex::new(crate::voice_observations::VoiceObservations::native(
        Arc::new(crate::native_plans::NativePlanReferences::default()),
    ));
    let failed = std::sync::atomic::AtomicBool::new(false);
    let ctx = SynthCtx {
        letter_navigation: false,
        gen: 1,
        gen_counter: &generation,
        cancellation: None,
        lifecycle: &lifecycle,
        engine: engine.as_ref(),
        control: &control,
        playback_tickets: Some(&tickets),
        presentation_clock: None,
        pending_overlays: None,
        timeline_renderer: Some(&renderer),
        effect_processor: Some(&effects),
        marker_span_id: None,
        marker_dispatch: None,
        voice_observations: Some(&observations),
        batch_failed: Some(&failed),
    };
    let timeline = PresentationTimelineV4 {
        protocol_version: 5,
        generation: 1,
        dispatch_id: 1,
        registry_generation: 41,
        delivery_policy: PresentationDeliveryPolicy::Ordered,
        replacement_key: None,
        // Markdown markup reaches synthesis as its own whitespace-only span.
        spans: ["Install the ", " ", "Omnivox speech server", " "]
            .into_iter()
            .enumerate()
            .map(|(index, text)| MixedSpeechSpan::EngineLayered(LayeredSpeechSpan {
                id: index as u64 + 1,
                text: text.into(),
                logical_voice_id: "bolden".into(),
                context: Default::default(),
                placement: Default::default(),
            }))
            .collect(),
        actions: vec![],
    };
    let state = omnivox_core::TtsState {
        punctuation_level: omnivox_core::PunctuationLevel::None,
        ..Default::default()
    };
    assert_eq!(
        process_presentation_timeline_v4(
            timeline, state, &ctx, &AudioFileLoader::new(), &engines,
            &RuntimeEngineHealth::default(), routing,
        ),
        BatchStatus::Completed,
    );
    assert!(!failed.load(Ordering::Acquire));
    let calls = engine.native_calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls[1].0.text.trim().is_empty());
    assert_eq!(calls[2].0.text, "Omnivox speech server");
    assert!(calls[3].0.text.trim().is_empty());
    let retained = std::mem::take(&mut *tickets.lock().unwrap());
    assert_eq!(retained.len(), 2, "silent spans must not create playback sources");
    for ticket in retained {
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }
}
#[test]
fn older_sinks_and_styles_cannot_discard_native_settings_or_evidence() {
    let engine = NativeEngine::new("dectalk", "Paul", true, None, ReceiptBehavior::Good);
    let mut engines = EngineRegistry::new();
    engines.register(engine.clone()).unwrap();
    let mut routing = snapshot(&engines, native_definition());
    let mut sink = NativeSink {
        supported: false,
        ..Default::default()
    };
    assert!(matches!(
        execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &[], UnavailablePolicy::CommonOnly),
            &mut sink,
            None
        ),
        PreparedSynthesisOutcome::Failed
    ));
    sink.supported = true;
    let settings = TtsSettings::default();
    let legacy = AttemptStyle::Legacy {
        settings: &settings,
        acss: None,
        effects: None,
    };
    assert!(matches!(
        execute(&engines, &mut routing, &legacy, &mut sink, None),
        PreparedSynthesisOutcome::Failed
    ));
    let mut route = routing.initial_native_route("bolden", &engines).unwrap();
    assert!(matches!(
        synthesize_with_runtime_fallback(
            "text",
            &settings,
            &mut route,
            &mut routing,
            &engines,
            &RuntimeEngineHealth::default(),
            1,
            &AtomicU64::new(1),
            None
        ),
        RuntimeSynthesisOutcome::Failed
    ));
    assert!(engine.inner.calls.lock().unwrap().is_empty());
}
#[test]
fn queue_budget_includes_native_payload_and_dispatch_snapshot_is_immutable() {
    let engines = EngineRegistry::new();
    let definition = native_definition();
    let mut small = definition.clone();
    small.choices.iter_mut().for_each(|c| c.native = None);
    let small = snapshot(&engines, small);
    let frozen = snapshot(&engines, definition.clone());
    assert!(frozen.queued_payload_bytes() > small.queued_payload_bytes());
    let mut larger = definition;
    let patch = larger.choices[0].native.as_mut().unwrap();
    for i in 0..60 {
        patch.parameters.insert(
            format!("future{i}"),
            Adjustment::Set {
                value: NativeValue::Enum("x".repeat(100)),
            },
        );
    }
    let larger = snapshot(&engines, larger);
    assert!(larger.queued_payload_bytes() > frozen.queued_payload_bytes() + 6000);
    assert_eq!(
        frozen.engine_definitions[0].choices[0]
            .native
            .as_ref()
            .unwrap()
            .parameters
            .len(),
        2
    );
}

#[test]
fn policy_fallback_executes_common_style_without_a_native_block_or_receipt() {
    let primary = NativeEngine::new(
        "dectalk",
        "Paul",
        true,
        Some(MockFailure::StreamBeforeAudio),
        ReceiptBehavior::Good,
    );
    let fallback = NativeEngine::new("espeak", "en", true, None, ReceiptBehavior::Good);
    let mut engines = EngineRegistry::new();
    engines.register(primary).unwrap();
    engines.register(fallback.clone()).unwrap();
    let mut definition = native_definition();
    definition.choices.truncate(1);
    let mut routing = snapshot(&engines, definition);
    routing.fallback_policy.fallback_engines = vec!["espeak".into()];
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    let mut sink = NativeSink::default();
    let a = actual(
        execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut sink,
            None,
        ),
        &sink,
    );
    assert_eq!(a.choice_id, None);
    assert_eq!(a.native, NativeChoiceExecution::NotRequested);
    assert!(a.native_application.is_none());
    assert_eq!(a.acss.style.average_pitch, Some(0.4));
    assert_eq!(a.effects.style.low_pass, Some(0.75));
    assert!(fallback.native_calls.lock().unwrap().is_empty());
    assert_eq!(fallback.inner.calls.lock().unwrap().len(), 1);
}

#[test]
fn adapter_common_only_is_preserved_when_permitted_and_buffered_stale_receipts_are_rejected() {
    let catalogues = [metadata("dectalk", "Paul"), metadata("eloquence", "Reed")];
    let knowledge = catalogues
        .iter()
        .map(ParameterKnowledge::Ready)
        .collect::<Vec<_>>();
    for streaming in [false, true] {
        let engine = NativeEngine::new(
            "dectalk",
            "Paul",
            streaming,
            None,
            ReceiptBehavior::CommonOnly,
        );
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut sink = NativeSink::default();
        let a = actual(
            execute(
                &engines,
                &mut routing,
                &style(
                    &Default::default(),
                    &knowledge,
                    UnavailablePolicy::CommonOnly,
                ),
                &mut sink,
                None,
            ),
            &sink,
        );
        assert_eq!(
            a.native_application.unwrap().status,
            ApplicationStatus::CommonOnly
        );
        assert_eq!(engine.native_calls.lock().unwrap().len(), 1);
    }
    let primary = NativeEngine::new("dectalk", "Paul", false, None, ReceiptBehavior::Stale);
    let fallback = NativeEngine::new("eloquence", "Reed", false, None, ReceiptBehavior::Good);
    let mut engines = EngineRegistry::new();
    engines.register(primary).unwrap();
    engines.register(fallback).unwrap();
    let mut routing = snapshot(&engines, native_definition());
    let mut sink = NativeSink::default();
    let a = actual(
        execute(
            &engines,
            &mut routing,
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut sink,
            None,
        ),
        &sink,
    );
    assert_eq!(a.resolution.realized.engine_id, "eloquence");
}

#[test]
fn native_output_failures_never_retry_or_quarantine_an_engine() {
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    for preflight in [false, true] {
        let primary = NativeEngine::new("dectalk", "Paul", true, None, ReceiptBehavior::Good);
        let fallback = NativeEngine::new("eloquence", "Reed", true, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(primary.clone()).unwrap();
        engines.register(fallback.clone()).unwrap();
        let mut routing = snapshot(&engines, native_definition());
        let mut route = routing.initial_native_route("bolden", &engines).unwrap();
        let health = RuntimeEngineHealth::default();
        let mut sink = NativeSink::default();
        sink.inner.fail_preflight = preflight;
        sink.inner.fail_start = !preflight;
        let result = synthesize_prepared_with_runtime_fallback_anchored(
            "test",
            &[],
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut route,
            &mut routing,
            &engines,
            &health,
            1,
            &AtomicU64::new(1),
            None,
            &mut sink,
        );
        assert!(matches!(result, PreparedSynthesisOutcome::Failed));
        assert!(matches!(
            health.acquire("dectalk"),
            EngineAccess::Permit(EnginePermit::Normal)
        ));
        assert!(fallback.inner.calls.lock().unwrap().is_empty());
        assert_eq!(
            primary.native_calls.lock().unwrap().len(),
            usize::from(!preflight)
        );
    }
}

/// Explicit opt-in because qualified proprietary runtimes are not CI dependencies.
#[test]
#[ignore = "requires OMNIVOX_NATIVE_ROUTING_CASES with qualified helper paths"]
fn qualified_helpers_route_native_choices_without_playback() {
    use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};
    use omnivox_tts::helper_engine::{HelperEngineConfig, HelperTtsEngine};
    use omnivox_tts::helper_protocol::parameters::{
        CatalogueAssembly, CatalogueQuery, CatalogueResult, Evidence, ExplanationResult,
        ExplanationSource,
    };
    #[derive(serde::Deserialize)]
    struct Case {
        engine: String,
        program: String,
        arguments: Vec<String>,
        voice: String,
        parameter: String,
        value: i64,
    }
    let cases: Vec<Case> = serde_json::from_slice(
        &std::fs::read(
            std::env::var("OMNIVOX_NATIVE_ROUTING_CASES").expect("explicit probe configuration"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut config = HelperEngineConfig::new(&case.engine, &case.program);
        config.arguments = case.arguments.into_iter().map(Into::into).collect();
        let helper = Arc::new(HelperTtsEngine::new(config).unwrap());
        let mut assembly = CatalogueAssembly::new(CatalogueQuery {
            engine_id: case.engine.clone(),
            voice_id: None,
            cursor: None,
            expected_catalogue_revision: None,
        })
        .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while let Some(query) = assembly.next_query().cloned() {
            match helper.query_parameters(query.clone()).unwrap() {
                page @ CatalogueResult::Ready { .. } => assembly.push(&query, &page).unwrap(),
                CatalogueResult::Busy { retry_after_ms } if Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(u64::from(retry_after_ms)))
                }
                other => panic!("catalogue unavailable: {other:?}"),
            }
        }
        let catalogue = assembly.finish().unwrap();
        let eligibility = Arc::new(omnivox_tts::voice_library::VoiceEligibility::default());
        let wrapped = Arc::new(IsolatedTtsEngine::new(
            eligibility.guard_engine(helper.clone()),
            Arc::new(AtomicU64::new(1)),
            Arc::new(IsolationBudget::new()),
        ));
        let primary = NativeEngine::new(
            "probe-failure",
            "first",
            true,
            Some(MockFailure::StreamBeforeAudio),
            ReceiptBehavior::Good,
        );
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(wrapped).unwrap();
        // These qualified catalogues fit one page; synthetic cache tests cover paging.
        assert!(
            catalogue.parameters.len()
                <= omnivox_tts::helper_protocol::parameters::MAX_PAGE_PARAMETERS
        );
        let queries = crate::parameter_queries::ParameterQueries::new();
        let query_payload =
            omnivox_tts::control::encode_request(&omnivox_tts::control::ControlRequestEnvelope {
                protocol_version: 1,
                request_id: 701,
                request: omnivox_tts::control::ControlRequest::GetEngineParametersV1(
                    CatalogueQuery {
                        engine_id: case.engine.clone(),
                        voice_id: None,
                        cursor: None,
                        expected_catalogue_revision: None,
                    },
                ),
            })
            .unwrap();
        assert!(queries.try_handle(&query_payload, &engines, &[]));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let cached = loop {
            let cached = queries.cached_catalogues(&engines, &[]);
            if !cached.is_empty() {
                break cached;
            }
            assert!(
                Instant::now() < deadline,
                "current catalogue was not cached"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(*cached[0], catalogue);
        let definition=EngineLayeredVoiceDefinition { id:"bolden".into(),language:None,shared:Default::default(),choices:vec![
            EngineVoiceChoice { id:"failed".into(),selector:exact("probe-failure","first"),adjustments:Default::default(),native:None },
            EngineVoiceChoice { id:"native".into(),selector:exact(&case.engine,&case.voice),adjustments:Default::default(),native:Some(serde_json::from_value(json!({"engine_id":case.engine,"schema_id":catalogue.identity.schema_id,"parameters":{case.parameter.clone():{"op":"set","value":case.value}}})).unwrap()) },
        ] };
        let knowledge = [ParameterKnowledge::Ready(&cached[0])];
        let registration = omnivox_tts::control::ControlRequestEnvelope {
            protocol_version: 1,
            request_id: 702,
            request: omnivox_tts::control::ControlRequest::RegisterLogicalVoicesV3(
                VoiceRegistrationV3 {
                    registry_generation: 41,
                    definitions: vec![EngineRegisteredVoiceDefinition::EngineLayered(definition)],
                    fallback_policy: omnivox_tts::control::ChoiceFallbackPolicy {
                        preferred_engines: vec![],
                        allow_same_language_on_requested_engine: false,
                        global_default: None,
                        fallback_engines: vec![],
                    },
                },
            ),
        };
        let mut registry = LogicalVoiceRegistry::default();
        let mut policy = omnivox_tts::routing_policy::RoutingPolicyRegistry::new("");
        let registered = omnivox_tts::control::process_control_request_with_parameters(
            &omnivox_tts::control::encode_request(&registration).unwrap(),
            "test",
            12,
            "",
            &engines.inventory(),
            &[],
            &mut registry,
            &mut policy,
            None,
            &knowledge,
        );
        let omnivox_tts::control::ControlResponse::LogicalVoicesRegisteredV3 {
            ref native_status,
            ..
        } = registered.response
        else {
            panic!("{registered:?}")
        };
        assert_eq!(native_status.len(), 1);
        assert_eq!(
            native_status[0].status,
            omnivox_tts::engine_voice_choices::NativeSupport::Supported
        );
        println!(
            "NATIVE_REGISTRATION {}",
            serde_json::to_string(&registered).unwrap()
        );
        for exact_preview in [false, true] {
            let mut routing =
                LogicalVoiceRoutingSnapshot::capture_with_policy(&registry, &engines, &policy);
            if exact_preview {
                routing.restrict_preview_to_choice(1).unwrap();
            }
            let mut sink = NativeSink::default();
            let outcome = execute(
                &engines,
                &mut routing,
                &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
                &mut sink,
                None,
            );
            let PreparedSynthesisOutcome::Streamed(completion) = outcome else {
                panic!("expected native progressive routing")
            };
            assert!(completion.frame_count > 0);
            assert_eq!(sink.inner.attempts.len(), 1);
            let attempt = &sink.inner.attempts[0];
            assert_eq!(attempt.choice_id.as_deref(), Some("native"));
            let receipt = attempt.native_application.as_ref().unwrap();
            assert_eq!(receipt.status, ApplicationStatus::Applied);
            let explanation = helper
                .explain_parameters(ExplanationSource::Applied {
                    plan_id: receipt.plan_id.clone().unwrap(),
                })
                .unwrap();
            let ExplanationResult::Ready {
                evidence,
                parameters,
                ..
            } = explanation
            else {
                panic!("missing applied explanation")
            };
            assert_eq!(evidence, Evidence::AdapterApplied);
            let parameter = parameters.iter().find(|p| p.id == case.parameter).unwrap();
            assert!(parameter.read_back);
            assert_eq!(parameter.value, Some(NativeValue::Integer(case.value)));
            println!(
                "NATIVE_ROUTING {}",
                json!({"engine":case.engine,"exact_preview":exact_preview,"choice_id":attempt.choice_id,"frames":completion.frame_count,"identity":receipt.identity,"native_parameter":parameter})
            );
        }
        helper.prepare_recovery_probe().unwrap();
        assert!(queries.cached_catalogues(&engines, &[]).is_empty());
        println!(
            "NATIVE_CACHE {}",
            json!({"engine":case.engine,"cached_parameters":cached[0].parameters.len(),"identity":cached[0].identity,"expired_after_recovery":true})
        );
    }
}

#[test]
fn superseded_generation_suppresses_native_preamble_and_pcm() {
    let generation = Arc::new(AtomicU64::new(1));
    let mut primary = NativeEngine::new("dectalk", "Paul", true, None, ReceiptBehavior::Good);
    Arc::get_mut(&mut primary).unwrap().supersede = Some(generation.clone());
    let mut engines = EngineRegistry::new();
    engines.register(primary).unwrap();
    let mut routing = snapshot(&engines, native_definition());
    let mut route = routing.initial_native_route("bolden", &engines).unwrap();
    let c = metadata("dectalk", "Paul");
    let knowledge = [ParameterKnowledge::Ready(&c)];
    let mut sink = NativeSink::default();
    assert!(matches!(
        synthesize_prepared_with_runtime_fallback_anchored(
            "test",
            &[],
            &style(&Default::default(), &knowledge, UnavailablePolicy::Require),
            &mut route,
            &mut routing,
            &engines,
            &RuntimeEngineHealth::default(),
            1,
            &generation,
            None,
            &mut sink
        ),
        PreparedSynthesisOutcome::Cancelled
    ));
    assert!(sink.inner.attempts.is_empty());
    assert!(sink.inner.stream.audio.is_empty());
}

include!("native_timeline_tests.rs");
