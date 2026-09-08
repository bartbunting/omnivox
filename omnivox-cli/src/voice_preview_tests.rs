//! Complete preview admission and synthesis tests using bounded simulated audio.
use super::*;
use omnivox_audio::{AudioBackend, AudioStreams};
use omnivox_tts::contracts::*;
use omnivox_tts::control::{decode_response, VoicePreviewPolicy};
use omnivox_tts::VoiceQuality;
use omnivox_tts::{
    SynthesisRequest, SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink,
    SynthesisStreamStart, TtsError, VoiceInfo,
};

#[derive(Clone, Copy)]
enum Behavior {
    Buffered,
    BufferedFailBeforeAudio,
    Empty,
    FailBeforeAudio,
    Stream,
    FailAfterAudio,
}

struct PreviewEngine {
    descriptor: EngineDescriptor,
    behavior: Behavior,
    requests: Mutex<Vec<SynthesisRequest>>,
}

impl PreviewEngine {
    fn new(id: &str, voice: &str, behavior: Behavior) -> Arc<Self> {
        Arc::new(Self {
            descriptor: EngineDescriptor {
                id: id.to_owned(),
                display_name: id.to_owned(),
                version: None,
                availability: Availability::Available,
                health: EngineHealth::Healthy,
                capabilities: EngineCapabilities {
                    acss: AcssCapabilities {
                        rate: true,
                        average_pitch: true,
                        pitch_range: true,
                        stress: true,
                        richness: true,
                        volume: true,
                    },
                    audio_output: if matches!(
                        behavior,
                        Behavior::Buffered | Behavior::BufferedFailBeforeAudio | Behavior::Empty
                    ) {
                        AudioOutputMode::BufferedPcm
                    } else {
                        AudioOutputMode::StreamingPcm
                    },
                    cancellation: CancellationSupport::PlaybackOnly,
                    concurrency: ConcurrencyModel::Serialized,
                    markers: MarkerCapabilities::default(),
                    language_switching: false,
                    text_repertoire: TextRepertoire::Unicode,
                    post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
                    native_extensions: Vec::new(),
                },
                voices: vec![VoiceDescriptor {
                    id: PhysicalVoiceId::new(id, voice),
                    display_name: voice.to_owned(),
                    language: Some("en-AU".to_owned()),
                    gender: None,
                    quality: VoiceQuality::Compact,
                    availability: Availability::Available,
                }],
                default_voice_id: Some(voice.to_owned()),
            },
            behavior,
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl TtsEngine for PreviewEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.requests.lock().unwrap().push(request.clone());
        if matches!(self.behavior, Behavior::BufferedFailBeforeAudio) {
            return Err(TtsError::SynthesisFailed(
                "before buffered audio".to_owned(),
            ));
        }
        Ok(SynthesisResult::audio(
            self.descriptor.id.clone(),
            request.requested_voice.clone(),
            if matches!(self.behavior, Behavior::Empty) {
                AudioBuffer::empty()
            } else {
                AudioBuffer::new(vec![0.25; 1024])
            },
        ))
    }
    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.requests.lock().unwrap().push(request.clone());
        sink.start(SynthesisStreamStart {
            engine_id: self.descriptor.id.clone(),
            actual_voice: request.requested_voice.clone(),
            degraded_acss: Vec::new(),
        })?;
        if matches!(self.behavior, Behavior::FailBeforeAudio) {
            return Err(TtsError::SynthesisFailed("before audio".to_owned()));
        }
        sink.audio(AudioBuffer::new(vec![0.25; 1024]))?;
        if matches!(self.behavior, Behavior::FailAfterAudio) {
            return Err(TtsError::SynthesisFailed("after audio".to_owned()));
        }
        Ok(SynthesisStreamCompletion { frame_count: 512 })
    }
    fn stop(&self) {}
    fn is_speaking(&self) -> bool {
        false
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        Vec::new()
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        None
    }
}

fn layered_request() -> VoicePreviewRequestV2 {
    use omnivox_tts::control::ChoiceFallbackPolicy;
    use omnivox_tts::voice_choices::{Adjustment, SharedAcss, SharedVoiceStyle, VoiceChoice};
    use omnivox_tts::voice_preview_v2::{PrivatePreviewVoice, VoicePreviewSelection};
    VoicePreviewRequestV2 {
        text: "A preview.".to_owned(),
        voice: PrivatePreviewVoice {
            language: Some("en-AU".to_owned()),
            shared: SharedVoiceStyle {
                acss: SharedAcss {
                    average_pitch: Some(0.4),
                    richness: Some(0.5),
                    ..Default::default()
                },
                rate_offset: Some(2),
                ..Default::default()
            },
            choices: [
                ("primary", "first", "one", 0.2, 0.8, 4),
                ("fallback", "second", "two", 0.6, 0.3, -1),
                ("soft", "second", "two", 0.9, 1.0, 0),
            ]
            .into_iter()
            .map(|(id, engine, voice, pitch, richness, offset)| VoiceChoice {
                id: id.to_owned(),
                selector: VoiceSelector::Exact(PhysicalVoiceId::new(engine, voice)),
                adjustments: VoiceStylePatch {
                    average_pitch: Some(Adjustment::Set { value: pitch }),
                    richness: Some(Adjustment::Set { value: richness }),
                    rate_offset: Some(Adjustment::Set { value: offset }),
                    ..Default::default()
                },
            })
            .collect(),
        },
        context: VoiceStylePatch::default(),
        placement: VoicePlacement::default(),
        selection: VoicePreviewSelection::Automatic {},
        fallback_policy: ChoiceFallbackPolicy {
            preferred_engines: vec!["second".to_owned()],
            allow_same_language_on_requested_engine: true,
            global_default: Some(VoiceSelector::Exact(PhysicalVoiceId::new("second", "two"))),
            fallback_engines: vec!["second".to_owned()],
        },
        disabled_engine_ids: Vec::new(),
        expected_base_rate: Some(0.65),
    }
}

fn run_layered(
    prepared: PreparedVoicePreviewV2,
    engines: &EngineRegistry,
    stale: bool,
) -> VoicePreviewResponseV2 {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let PreviewTarget::Layered { base_rate, .. } = &prepared.target else {
        panic!("wrong target")
    };
    let state = TtsState {
        speech_rate: *base_rate,
        current_voice: "unrelated".to_owned(),
        pitch_multiplier: 1.8,
        ..Default::default()
    };
    let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
    let control = streams.control();
    let engine = engines.engine(&engines.inventory()[0].id).unwrap();
    let mut owned_engines = EngineRegistry::new();
    for descriptor in engines.inventory() {
        owned_engines
            .register(engines.engine(&descriptor.id).unwrap())
            .unwrap();
    }
    let captured = Capture(Arc::new(Mutex::new(Vec::new())));
    let (output, writer) =
        crate::marker_events::spawn_marker_event_reporter_with_writer(captured.clone());
    let (sender, tracker) = spawn_tracked_playback_reporter(output.clone());
    let (work_sender, receiver) = synthesis_channel();
    assert!(enqueue_synthesis(
        &work_sender,
        SynthRequest::Preview {
            request_id: 702,
            text: prepared.text,
            requested: prepared.target,
            state,
            logical_voice_routing: prepared.routing,
            lifecycle: RequestLifecycle::default(),
            gen: 1,
        }
    ));
    drop(work_sender);
    synthesis_worker(
        receiver,
        Arc::new(AtomicU64::new(if stale { 2 } else { 1 })),
        engine,
        Arc::new(owned_engines),
        Arc::new(RuntimeEngineHealth::new()),
        control.clone(),
        AudioFileLoader::with_cache(),
        sender,
        output,
    );
    tracker.join().unwrap();
    writer.join().unwrap();
    control.drain();
    let records = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(records.lines().count(), 1);
    let envelope = decode_record(&records);
    assert_eq!(envelope.request_id, Some(702));
    let ControlResponse::PreviewVoiceCompletedV2(response) = envelope.response else {
        panic!("wrong response")
    };
    response
}

#[test]
fn layered_preview_fallback_uses_actual_patch_in_all_output_mode_combinations() {
    use omnivox_tts::voice_choices::Adjustment;
    for primary in [Behavior::BufferedFailBeforeAudio, Behavior::FailBeforeAudio] {
        for fallback in [Behavior::Buffered, Behavior::Stream] {
            let first = PreviewEngine::new("first", "one", primary);
            let second = PreviewEngine::new("second", "two", fallback);
            let mut engines = EngineRegistry::new();
            engines.register(first.clone()).unwrap();
            engines.register(second.clone()).unwrap();
            let mut draft = layered_request();
            draft.context.richness = Some(Adjustment::Set { value: 0.5 }); // Same as shared still wins.
            let prepared = prepare_voice_preview_v2(
                draft,
                0.65,
                &engines,
                &RoutingPolicyRegistry::new("first"),
            )
            .unwrap();
            let response = run_layered(prepared, &engines, false);
            assert_eq!(response.status, PreviewStatus::Completed);
            assert_eq!(response.accepted_audio.len(), 1);
            let audio = &response.accepted_audio[0];
            assert_eq!(audio.choice_id.as_deref(), Some("fallback"));
            assert_eq!(
                audio.reason,
                omnivox_tts::resolver::ResolutionReason::ExplicitAlternative {
                    preference_index: 1
                }
            );
            assert!(audio.playback_started);
            assert_eq!(
                response.last_started.unwrap().realized,
                PhysicalVoiceId::new("second", "two")
            );
            let requests = second.requests.lock().unwrap();
            assert!(!requests.is_empty());
            for request in requests.iter() {
                assert!((request.settings.rate - 0.64).abs() < 0.00001);
                assert_eq!(request.normalized_acss.average_pitch, Some(0.6));
                assert_eq!(request.normalized_acss.richness, Some(0.5));
            }
        }
    }
}

#[test]
fn layered_individual_preview_keeps_duplicate_row_identity_and_cannot_substitute() {
    use omnivox_tts::voice_preview_v2::VoicePreviewSelection;
    let first = PreviewEngine::new("first", "one", Behavior::Buffered);
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(first.clone()).unwrap();
    engines.register(second.clone()).unwrap();
    let policy = RoutingPolicyRegistry::new("first");
    let mut draft = layered_request();
    draft.selection = VoicePreviewSelection::Choice {
        choice_id: "soft".to_owned(),
    };
    let response = run_layered(
        prepare_voice_preview_v2(draft.clone(), 0.65, &engines, &policy).unwrap(),
        &engines,
        false,
    );
    let last = response.last_started.unwrap();
    assert_eq!(last.choice_id.as_deref(), Some("soft"));
    assert_eq!(
        last.reason,
        omnivox_tts::resolver::ResolutionReason::ExplicitAlternative {
            preference_index: 2
        }
    );
    assert!(first.requests.lock().unwrap().is_empty());
    assert_eq!(
        second.requests.lock().unwrap()[0].normalized_acss.richness,
        Some(1.0)
    );
    assert_eq!(second.requests.lock().unwrap()[0].settings.rate, 0.65);
    draft.disabled_engine_ids = vec!["second".to_owned()];
    let response = run_layered(
        prepare_voice_preview_v2(draft, 0.65, &engines, &policy).unwrap(),
        &engines,
        false,
    );
    assert_eq!(response.status, PreviewStatus::Failed);
    assert!(response.accepted_audio.is_empty());
    assert!(response.last_started.is_none());
    assert!(first.requests.lock().unwrap().is_empty());
}

#[test]
fn layered_policy_fallback_uses_shared_without_a_choice_patch() {
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(second.clone()).unwrap();
    let mut draft = layered_request();
    draft.voice.choices.clear();
    let response = run_layered(
        prepare_voice_preview_v2(draft, 0.65, &engines, &RoutingPolicyRegistry::new("first"))
            .unwrap(),
        &engines,
        false,
    );
    assert_eq!(response.status, PreviewStatus::Completed);
    assert_eq!(response.last_started.unwrap().choice_id, None);
    let requests = second.requests.lock().unwrap();
    assert_eq!(requests[0].normalized_acss.average_pitch, Some(0.4));
    assert!((requests[0].settings.rate - 0.67).abs() < 0.00001);
}

#[test]
fn layered_preview_default_and_zero_restore_captured_host_rate_above_one() {
    use omnivox_tts::voice_choices::Adjustment;
    use omnivox_tts::voice_preview_v2::VoicePreviewSelection;
    for reset in [Adjustment::Default {}, Adjustment::Set { value: 0 }] {
        let second = PreviewEngine::new("second", "two", Behavior::Buffered);
        let mut engines = EngineRegistry::new();
        engines.register(second.clone()).unwrap();
        let mut draft = layered_request();
        draft.expected_base_rate = Some(1.4);
        draft.selection = VoicePreviewSelection::Choice {
            choice_id: "soft".to_owned(),
        };
        draft.context.average_pitch = Some(Adjustment::Default {});
        draft.context.rate_offset = Some(reset);
        let response = run_layered(
            prepare_voice_preview_v2(draft, 1.4, &engines, &RoutingPolicyRegistry::new("first"))
                .unwrap(),
            &engines,
            false,
        );
        assert_eq!(response.status, PreviewStatus::Completed);
        assert_eq!(response.base_rate, 1.4);
        let requests = second.requests.lock().unwrap();
        assert_eq!(requests[0].settings.rate, 1.4);
        assert_eq!(requests[0].settings.pitch, 1.0);
        assert_eq!(requests[0].normalized_acss.average_pitch, None);
    }
}

#[test]
fn layered_preview_empty_stale_and_failed_stream_terminals_are_truthful() {
    for (behavior, stale) in [
        (Behavior::Empty, false),
        (Behavior::Buffered, true),
        (Behavior::FailAfterAudio, false),
    ] {
        let first = PreviewEngine::new("first", "one", behavior);
        let second = PreviewEngine::new("second", "two", Behavior::Buffered);
        let mut engines = EngineRegistry::new();
        engines.register(first.clone()).unwrap();
        engines.register(second.clone()).unwrap();
        let response = run_layered(
            prepare_voice_preview_v2(
                layered_request(),
                0.65,
                &engines,
                &RoutingPolicyRegistry::new("first"),
            )
            .unwrap(),
            &engines,
            stale,
        );
        if stale {
            assert_eq!(response.status, PreviewStatus::Cancelled);
            assert!(first.requests.lock().unwrap().is_empty());
            assert!(response.accepted_audio.is_empty());
        } else if matches!(behavior, Behavior::Empty) {
            assert_eq!(response.status, PreviewStatus::Completed);
            assert!(response.accepted_audio.is_empty());
            assert!(response.last_started.is_none());
        } else {
            assert_eq!(response.status, PreviewStatus::Failed);
            assert_eq!(response.accepted_audio.len(), 1);
            assert_eq!(
                response.accepted_audio[0].choice_id.as_deref(),
                Some("primary")
            );
            assert!(second.requests.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn layered_preview_freezes_private_policy_disable_union_and_full_queue_payload() {
    let first = PreviewEngine::new("first", "one", Behavior::Buffered);
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(first.clone()).unwrap();
    engines.register(second.clone()).unwrap();
    let mut policy = RoutingPolicyRegistry::new("first");
    policy
        .register(
            1,
            RoutingPolicy {
                disabled_engine_ids: vec!["first".to_owned(), "admin".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
    let mut draft = layered_request();
    draft.disabled_engine_ids = vec!["client".to_owned(), "admin".to_owned()];
    let prepared = prepare_voice_preview_v2(draft.clone(), 0.65, &engines, &policy).unwrap();
    policy
        .register(
            2,
            RoutingPolicy {
                disabled_engine_ids: vec!["second".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
    let response = run_layered(prepared, &engines, false);
    assert_eq!(response.status, PreviewStatus::Completed);
    assert_eq!(
        response.effective_disabled_engine_ids,
        vec!["client", "admin", "first"]
    );
    assert_eq!(
        response.last_started.unwrap().choice_id.as_deref(),
        Some("fallback")
    );
    assert!(first.requests.lock().unwrap().is_empty());
    assert_eq!(policy.generation(), 2);
    assert_eq!(policy.policy().disabled_engine_ids, vec!["second"]);
    let payload = |draft| {
        let prepared = prepare_voice_preview_v2(draft, 0.65, &engines, &policy).unwrap();
        SynthRequest::Preview {
            request_id: 702,
            text: prepared.text,
            requested: prepared.target,
            state: TtsState::default(),
            logical_voice_routing: prepared.routing,
            lifecycle: RequestLifecycle::default(),
            gen: 1,
        }
        .queued_payload_bytes()
    };
    // Even strict selection must retain/account for the other authoritative rows.
    draft.selection = omnivox_tts::voice_preview_v2::VoicePreviewSelection::Choice {
        choice_id: "primary".to_owned(),
    };
    let small = payload(draft.clone());
    draft.voice.choices[2].selector =
        VoiceSelector::Exact(PhysicalVoiceId::new("second", "x".repeat(4096)));
    assert!(payload(draft) >= small + 4000);
}

#[test]
fn layered_individual_runtime_failure_does_not_use_automatic_policy() {
    let first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(first.clone()).unwrap();
    engines.register(second.clone()).unwrap();
    let mut draft = layered_request();
    draft.selection = omnivox_tts::voice_preview_v2::VoicePreviewSelection::Choice {
        choice_id: "primary".to_owned(),
    };
    let result = run_layered(
        prepare_voice_preview_v2(draft, 0.65, &engines, &RoutingPolicyRegistry::new("first"))
            .unwrap(),
        &engines,
        false,
    );
    assert_eq!(result.status, PreviewStatus::Failed);
    assert!(result.accepted_audio.is_empty());
    assert!(result.last_started.is_none());
    assert_eq!(first.requests.lock().unwrap().len(), 1);
    assert!(second.requests.lock().unwrap().is_empty());
}

#[test]
fn layered_preview_rejects_unreportable_actual_voice_before_synthesis() {
    let first = PreviewEngine::new("first", &"x".repeat(32 * 1024), Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(first.clone()).unwrap();
    let mut draft = layered_request();
    draft.voice.choices[0].selector = VoiceSelector::EngineDefault {
        engine_id: "first".to_owned(),
    };
    let response = run_layered(
        prepare_voice_preview_v2(draft, 0.65, &engines, &RoutingPolicyRegistry::new("first"))
            .unwrap(),
        &engines,
        false,
    );
    assert_eq!(response.status, PreviewStatus::Failed);
    assert!(response.accepted_audio.is_empty());
    assert!(response.last_started.is_none());
    assert!(first.requests.lock().unwrap().is_empty());
}

fn request() -> VoicePreviewRequest {
    VoicePreviewRequest {
        text: "A preview.".to_owned(),
        preferences: Vec::new(),
        language: Some("en-AU".to_owned()),
        acss: NormalizedAcss::default(),
        rate_offset: None,
        effects: PostSynthesisStyle::default(),
        fallback_policy: VoicePreviewPolicy {
            preferred_engines: vec!["first".to_owned(), "second".to_owned()],
            allow_same_language_on_requested_engine: true,
            global_default: None,
            fallback_engines: Vec::new(),
        },
        disabled_engine_ids: Vec::new(),
        expected_base_rate: None,
    }
}

fn run(
    prepared: PreparedVoicePreview,
    engines: &EngineRegistry,
    stale: bool,
) -> PreviewSynthesisResult {
    let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
    let control = streams.control();
    let generation = AtomicU64::new(if stale { 2 } else { 1 });
    let lifecycle = RequestLifecycle::default();
    let engine = engines.engine(&engines.inventory()[0].id).unwrap();
    let tickets = Mutex::new(Vec::new());
    let renderer = Mutex::new(TimelineAudioRenderer::new());
    let effects = Mutex::new(crate::pipeline::DispatchEffects::new());
    let failed = AtomicBool::new(false);
    let ctx = SynthCtx {
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
        voice_observations: None,
        batch_failed: Some(&failed),
    };
    let mut result = process_preview(
        &prepared.text,
        TtsState {
            speech_rate: prepared.base_rate,
            // A legacy binding must not decide an Automatic draft.
            current_voice: "unrelated-legacy-voice".to_owned(),
            ..TtsState::default()
        },
        &ctx,
        engines,
        &RuntimeEngineHealth::new(),
        prepared.routing,
        PREVIEW_LOGICAL_VOICE_ID,
    );
    result.status = await_tracked_playback(result.status, tickets.into_inner().unwrap());
    control.drain();
    result
}

fn decode_record(record: &str) -> ControlResponseEnvelope {
    decode_response(record.split_whitespace().last().unwrap()).unwrap()
}

#[test]
fn complete_preview_executes_shared_wire_examples() {
    let wire: Vec<_> = include_str!("../../test-fixtures/voice-preview-wire.txt")
        .lines()
        .collect();
    for case in wire.chunks_exact(2) {
        let engine = PreviewEngine::new("eloquence", "Reed", Behavior::Buffered);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let mut policy = RoutingPolicyRegistry::new("unrelated-live-preference");
        policy
            .register(
                7,
                RoutingPolicy {
                    disabled_engine_ids: vec!["winrt".to_owned()],
                    ..RoutingPolicy::default()
                },
            )
            .unwrap();
        let before = policy.policy().clone();
        let envelope = decode_request(case[0]).unwrap();
        let ControlRequest::PreviewVoice(request) = envelope.request else {
            panic!()
        };
        let prepared = prepare_voice_preview(request, 0.65, &engines, &policy).unwrap();
        let disabled = prepared.disabled_engine_ids.clone();
        let result = run(prepared, &engines, false);
        let response = decode_record(&voice_preview_status_record(
            envelope.request_id,
            result,
            0.65,
            disabled,
        ));
        assert_eq!(response, decode_response(case[1]).unwrap());
        let calls = engine.requests.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!((calls[0].normalized_acss.rate.unwrap() - 0.69).abs() < 0.0001);
        assert_eq!(calls[0].normalized_acss.average_pitch, Some(0.0));
        assert_eq!(policy.generation(), 7);
        assert_eq!(policy.policy(), &before);
    }
}

#[test]
fn complete_preview_freezes_disablements_and_policy_at_admission() {
    let mut engines = EngineRegistry::new();
    let first = PreviewEngine::new("first", "one", Behavior::Buffered);
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    engines.register(first.clone()).unwrap();
    engines.register(second.clone()).unwrap();
    let mut policy = RoutingPolicyRegistry::new("first");
    policy
        .register(
            1,
            RoutingPolicy {
                disabled_engine_ids: vec!["first".to_owned()],
                ..RoutingPolicy::default()
            },
        )
        .unwrap();
    let mut draft = request();
    draft.disabled_engine_ids = vec!["other".to_owned()];
    let mut prepared = prepare_voice_preview(draft, 0.65, &engines, &policy).unwrap();
    assert_eq!(prepared.disabled_engine_ids, vec!["other", "first"]);
    policy
        .register(
            2,
            RoutingPolicy {
                disabled_engine_ids: vec!["second".to_owned()],
                ..RoutingPolicy::default()
            },
        )
        .unwrap();
    prepared.routing.replace_inventory(engines.inventory());
    let result = run(prepared, &engines, false);
    assert_eq!(
        result.evidence.realizations,
        vec![PhysicalVoiceId::new("second", "two")]
    );
    assert!(first.requests.lock().unwrap().is_empty());
}

#[test]
fn complete_preview_records_only_accepted_audio_across_fallback() {
    for second_behavior in [Behavior::Buffered, Behavior::Stream] {
        let mut engines = EngineRegistry::new();
        let first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
        let second = PreviewEngine::new("second", "two", second_behavior);
        engines.register(first.clone()).unwrap();
        engines.register(second.clone()).unwrap();
        let prepared = prepare_voice_preview(
            request(),
            0.65,
            &engines,
            &RoutingPolicyRegistry::new("first"),
        )
        .unwrap();
        let result = run(prepared, &engines, false);
        assert_eq!(result.status, BatchStatus::Completed);
        assert_eq!(
            result.evidence.realizations,
            vec![PhysicalVoiceId::new("second", "two")]
        );
        assert_eq!(first.requests.lock().unwrap().len(), 1);
        assert_eq!(second.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn complete_preview_keeps_partial_audio_failure_without_replay() {
    let mut engines = EngineRegistry::new();
    let first = PreviewEngine::new("first", "one", Behavior::FailAfterAudio);
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    engines.register(first.clone()).unwrap();
    engines.register(second.clone()).unwrap();
    let prepared = prepare_voice_preview(
        request(),
        0.65,
        &engines,
        &RoutingPolicyRegistry::new("first"),
    )
    .unwrap();
    let result = run(prepared, &engines, false);
    assert_eq!(result.status, BatchStatus::Failed);
    assert_eq!(
        result.evidence.realizations,
        vec![PhysicalVoiceId::new("first", "one")]
    );
    assert!(second.requests.lock().unwrap().is_empty());
}

#[test]
fn complete_preview_empty_and_cancelled_attempts_do_not_claim_audio() {
    let mut engines = EngineRegistry::new();
    let engine = PreviewEngine::new("first", "one", Behavior::Empty);
    engines.register(engine.clone()).unwrap();
    for stale in [false, true] {
        let prepared = prepare_voice_preview(
            request(),
            0.65,
            &engines,
            &RoutingPolicyRegistry::new("first"),
        )
        .unwrap();
        let result = run(prepared, &engines, stale);
        assert!(result.evidence.realizations.is_empty());
        assert!(result.evidence.degraded_acss.is_empty());
        assert_eq!(
            result.status,
            if stale {
                BatchStatus::Cancelled
            } else {
                BatchStatus::Completed
            }
        );
    }
    assert_eq!(engine.requests.lock().unwrap().len(), 1);
}

#[test]
fn complete_preview_rejects_invalid_data_without_mutation() {
    let engines = EngineRegistry::new();
    let policy = RoutingPolicyRegistry::new("live");
    for invalid in 0..8 {
        let mut draft = request();
        match invalid {
            0 => draft.text.clear(),
            1 => draft.text = "x".repeat(MAX_PREVIEW_TEXT_BYTES + 1),
            2 => {
                draft.preferences =
                    vec![VoiceSelector::Exact(PhysicalVoiceId::new("first", "one")); 33]
            }
            3 => draft.expected_base_rate = Some(0.5),
            4 => draft.expected_base_rate = Some(f32::NAN),
            5 => draft.disabled_engine_ids = vec!["".to_owned()],
            6 => draft.preferences = vec![VoiceSelector::Exact(PhysicalVoiceId::new("first", ""))],
            7 => {
                draft.acss.rate = Some(0.5);
                draft.rate_offset = Some(1);
            }
            _ => unreachable!(),
        }
        assert!(
            prepare_voice_preview(draft, 0.65, &engines, &policy).is_err(),
            "case {invalid}"
        );
        assert_eq!(policy.generation(), 0);
        assert!(engines.is_empty());
    }
}

#[test]
fn complete_preview_terminal_metadata_is_bounded_and_correlated() {
    let result = PreviewSynthesisResult {
        status: BatchStatus::Failed,
        realized: None,
        degraded_acss: Vec::new(),
        degraded_effects: Vec::new(),
        message: Some("x".repeat(omnivox_tts::control::MAX_CONTROL_PAYLOAD_BYTES)),
        evidence: Default::default(),
    };
    let response = decode_record(&voice_preview_status_record(41, result, 0.65, Vec::new()));
    assert_eq!(response.request_id, Some(41));
    assert!(matches!(
        response.response,
        ControlResponse::Error {
            code: ControlErrorCode::PayloadTooLarge,
            ..
        }
    ));
}

#[test]
fn complete_preview_failure_reports_last_target_and_only_accepted_degradation() {
    let mut engines = EngineRegistry::new();
    let mut first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
    Arc::get_mut(&mut first)
        .unwrap()
        .descriptor
        .capabilities
        .acss
        .richness = false;
    let second = PreviewEngine::new("second", "two", Behavior::FailAfterAudio);
    engines.register(first).unwrap();
    engines.register(second).unwrap();
    let mut draft = request();
    draft.acss.richness = Some(0.8);
    let prepared =
        prepare_voice_preview(draft, 0.65, &engines, &RoutingPolicyRegistry::new("first")).unwrap();
    let result = run(prepared, &engines, false);
    assert_eq!(result.status, BatchStatus::Failed);
    assert_eq!(result.realized, Some(PhysicalVoiceId::new("second", "two")));
    assert_eq!(
        result.evidence.realizations,
        vec![PhysicalVoiceId::new("second", "two")]
    );
    assert!(result.evidence.degraded_acss.is_empty());
}

#[test]
fn complete_preview_uses_the_same_route_as_an_equivalent_live_registration() {
    let mut engines = EngineRegistry::new();
    engines
        .register(PreviewEngine::new("first", "one", Behavior::Buffered))
        .unwrap();
    engines
        .register(PreviewEngine::new("second", "two", Behavior::Buffered))
        .unwrap();
    let mut policy = RoutingPolicyRegistry::new("first");
    policy
        .register(
            1,
            RoutingPolicy {
                preferred_engine_ids: vec!["second".to_owned()],
                ..RoutingPolicy::default()
            },
        )
        .unwrap();
    let mut draft = request();
    draft.acss.average_pitch = Some(0.8);
    draft.effects.reverb = Some(0.2);
    let mut live = LogicalVoiceRegistry::default();
    live.register(
        1,
        vec![LogicalVoiceDefinition {
            id: PREVIEW_LOGICAL_VOICE_ID.to_owned(),
            language: draft.language.clone(),
            preferences: draft.preferences.clone(),
            acss: draft.acss.clone(),
            effects: draft.effects.clone(),
        }],
        FallbackPolicy::default(),
        &engines.inventory(),
    )
    .unwrap();
    draft.fallback_policy.preferred_engines = vec!["second".to_owned()];
    let live_route = LogicalVoiceRoutingSnapshot::capture_with_policy(&live, &engines, &policy)
        .initial_route(PREVIEW_LOGICAL_VOICE_ID, &engines)
        .unwrap();
    let prepared = prepare_voice_preview(draft, 0.65, &engines, &policy).unwrap();
    let preview_route = prepared
        .routing
        .initial_route(PREVIEW_LOGICAL_VOICE_ID, &engines)
        .unwrap();
    assert_eq!(preview_route.realized, live_route.realized);
    assert_eq!(preview_route.acss, live_route.acss);
    assert_eq!(preview_route.effects, live_route.effects);
}

#[test]
fn complete_preview_queue_accounts_for_private_chain_and_policy() {
    let engines = EngineRegistry::new();
    let policy = RoutingPolicyRegistry::new("first");
    let queued = |request| {
        let prepared = prepare_voice_preview(request, 0.65, &engines, &policy).unwrap();
        SynthRequest::Preview {
            request_id: 1,
            text: prepared.text,
            requested: PreviewTarget::Complete {
                base_rate: prepared.base_rate,
                disabled_engine_ids: prepared.disabled_engine_ids,
            },
            state: TtsState::default(),
            logical_voice_routing: prepared.routing,
            lifecycle: RequestLifecycle::default(),
            gen: 1,
        }
    };
    let small = queued(request());
    let mut draft = request();
    draft.preferences = vec![VoiceSelector::Exact(PhysicalVoiceId::new(
        "first",
        "x".repeat(1000),
    ))];
    draft.disabled_engine_ids = vec!["disabled".to_owned()];
    assert!(queued(draft).queued_payload_bytes() >= small.queued_payload_bytes() + 1000);
}

#[test]
fn complete_preview_streaming_effects_follow_the_engine_that_accepts_audio() {
    for (first_supports, second_supports) in [(true, false), (false, true)] {
        let mut engines = EngineRegistry::new();
        let mut first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
        let mut second = PreviewEngine::new("second", "two", Behavior::Stream);
        if !first_supports {
            Arc::get_mut(&mut first)
                .unwrap()
                .descriptor
                .capabilities
                .post_synthesis_dimensions
                .clear();
        }
        if !second_supports {
            Arc::get_mut(&mut second)
                .unwrap()
                .descriptor
                .capabilities
                .post_synthesis_dimensions
                .clear();
        }
        engines.register(first).unwrap();
        engines.register(second).unwrap();
        let mut draft = request();
        draft.effects.pan = Some(0.25);
        let prepared =
            prepare_voice_preview(draft, 0.65, &engines, &RoutingPolicyRegistry::new("first"))
                .unwrap();
        let result = run(prepared, &engines, false);
        assert_eq!(result.status, BatchStatus::Completed);
        assert_eq!(
            result.evidence.degraded_effects,
            if second_supports {
                Vec::new()
            } else {
                vec![PostSynthesisDimension::Pan]
            }
        );
    }
}
