use super::*;
use omnivox_tts::native_parameters::ParameterCatalogue;
use omnivox_tts::native_synthesis::{ApplicationStatus, NativeApplication, VoiceParameters};
use omnivox_tts::voice_preview_v2::VoicePreviewSelection;
use omnivox_tts::voice_preview_v3::{VoicePreviewRequestV3, VoicePreviewResponseV3};
use serde_json::{json, Value};
#[derive(Clone, Copy)]
enum NativeBehavior {
    Good,
    CommonOnly,
    Stale,
}
struct NativePreviewEngine {
    inner: Arc<PreviewEngine>,
    native_behavior: NativeBehavior,
    parameters: Mutex<Vec<VoiceParameters>>,
    epoch: AtomicU64,
}
impl NativePreviewEngine {
    fn new(id: &str, voice: &str, behavior: Behavior) -> Arc<Self> {
        Arc::new(Self {
            inner: PreviewEngine::new(id, voice, behavior),
            native_behavior: NativeBehavior::Good,
            parameters: Mutex::new(vec![]),
            epoch: AtomicU64::new(1),
        })
    }
    fn receipt(&self, p: &VoiceParameters) -> NativeApplication {
        let mut application = NativeApplication {
            status: ApplicationStatus::Applied,
            plan_id: Some("helper-local-plan".into()),
            identity: Some(p.expected_identity.clone()),
            masked_parameters: vec![],
            reason: None,
        };
        match self.native_behavior {
            NativeBehavior::Good => (),
            NativeBehavior::Stale => application.identity.as_mut().unwrap().runtime_generation += 1,
            NativeBehavior::CommonOnly => {
                application = NativeApplication {
                    status: ApplicationStatus::CommonOnly,
                    plan_id: None,
                    identity: None,
                    masked_parameters: vec![],
                    reason: Some("unavailable".into()),
                }
            }
        }
        application
    }
}
impl TtsEngine for NativePreviewEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.inner.descriptor()
    }
    fn parameter_cache_epoch(&self) -> Option<u64> {
        Some(self.epoch.load(Ordering::Acquire))
    }
    fn synthesize(&self, r: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.inner.synthesize(r)
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
        r: &SynthesisRequest,
        p: &VoiceParameters,
    ) -> Result<(SynthesisResult, NativeApplication), TtsError> {
        self.parameters.lock().unwrap().push(p.clone());
        Ok((self.inner.synthesize(r)?, self.receipt(p)))
    }
    fn synthesize_stream_with_parameters(
        &self,
        r: &SynthesisRequest,
        p: &VoiceParameters,
        s: &mut dyn SynthesisStreamSink,
        a: &mut dyn FnMut(&NativeApplication),
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.parameters.lock().unwrap().push(p.clone());
        a(&self.receipt(p));
        self.inner.synthesize_stream(r, s)
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
fn fixture(name: &str) -> Value {
    serde_json::from_str::<Value>(include_str!(
        "../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()["messages"][name]
        .clone()
}
fn draft() -> VoicePreviewRequestV3 {
    let mut value = fixture("preview");
    for key in ["type", "protocol_version", "request_id"] {
        value.as_object_mut().unwrap().remove(key);
    }
    serde_json::from_value(value).unwrap()
}
fn catalogue(engine: &str, voice: &str) -> Arc<ParameterCatalogue> {
    let mut value = fixture("catalogue_response")["result"].clone();
    for key in ["status", "next_cursor"] {
        value.as_object_mut().unwrap().remove(key);
    }
    value["engine_id"] = json!(engine);
    value["voice_id"] = json!(voice);
    if engine == "eloquence" {
        value["identity"]["schema_id"] = json!("eloquence.eci-units.v1");
        value["parameters"] = json!([value["parameters"][1].clone()]);
        value["parameters"][0]["id"] = json!("breathiness");
        value["mappings"] =
            json!([{"common_inputs":["richness"],"native_outputs":["breathiness"]}]);
    }
    Arc::new(ParameterCatalogue::from_json(&serde_json::to_vec(&value).unwrap()).unwrap())
}
fn prepare(request: VoicePreviewRequestV3, engines: &EngineRegistry) -> PreparedChoicePreview {
    prepare_voice_preview_v3(
        request,
        0.65,
        engines,
        &RoutingPolicyRegistry::new("dectalk"),
        vec![catalogue("dectalk", "paul"), catalogue("eloquence", "v1")],
    )
    .unwrap()
}
fn run_native(
    prepared: PreparedChoicePreview,
    engines: &EngineRegistry,
    stale: bool,
) -> VoicePreviewResponseV3 {
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
    let PreviewTarget::Native { base_rate, .. } = &prepared.target else {
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
        Arc::new(crate::native_plans::NativePlanReferences::default()),
    );
    tracker.join().unwrap();
    writer.join().unwrap();
    control.drain();
    let records = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(records.lines().count(), 1);
    let envelope = decode_record(&records);
    assert_eq!(envelope.request_id, Some(702));
    let ControlResponse::PreviewVoiceCompletedV3(response) = envelope.response else {
        panic!("wrong response")
    };
    response
}

#[test]
fn native_preview_automatic_fallback_retains_actual_controls_in_all_audio_modes() {
    for buffered in [true, false] {
        for fallback_buffered in [true, false] {
            let first = NativePreviewEngine::new(
                "dectalk",
                "paul",
                if buffered {
                    Behavior::BufferedFailBeforeAudio
                } else {
                    Behavior::FailBeforeAudio
                },
            );
            let second = NativePreviewEngine::new(
                "eloquence",
                "v1",
                if fallback_buffered {
                    Behavior::Buffered
                } else {
                    Behavior::Stream
                },
            );
            let mut engines = EngineRegistry::new();
            engines.register(first.clone()).unwrap();
            engines.register(second.clone()).unwrap();
            let mut request = draft();
            request.selection = VoicePreviewSelection::Automatic {};
            request.context.richness =
                Some(omnivox_tts::voice_choices::Adjustment::Set { value: 0.5 });
            let response = run_native(prepare(request, &engines), &engines, false);
            assert_eq!(response.status, PreviewStatus::Completed);
            assert_eq!(response.accepted_audio.len(), 1);
            let evidence = &response.accepted_audio[0];
            assert_eq!(evidence.choice_id.as_deref(), Some("eci-default"));
            assert_eq!(evidence.realized.engine_id, "eloquence");
            assert!(evidence.playback_started);
            let application = evidence.native_application.as_ref().unwrap();
            assert_eq!(application.status, ApplicationStatus::Applied);
            assert_ne!(application.plan_id.as_deref(), Some("helper-local-plan"));
            assert_eq!(
                response.last_started.unwrap().native_application.as_ref(),
                Some(application)
            );
            let calls = second.parameters.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0].native.parameters["breathiness"],
                omnivox_tts::voice_choices::Adjustment::Set {
                    value: omnivox_tts::native_parameters::NativeValue::Integer(42)
                }
            );
            assert_eq!(
                calls[0].context_dimensions,
                vec![omnivox_tts::native_parameters::CommonInput::Richness]
            );
            assert_eq!(
                calls[0].unavailable_policy,
                omnivox_tts::native_synthesis::UnavailablePolicy::Require
            );
        }
    }
}
#[test]
fn native_preview_exact_duplicate_voice_choice_keeps_its_tuning_and_never_substitutes() {
    for behavior in [
        Behavior::Buffered,
        Behavior::Stream,
        Behavior::BufferedFailBeforeAudio,
        Behavior::FailBeforeAudio,
    ] {
        let engine = NativePreviewEngine::new("dectalk", "paul", behavior);
        let fallback = NativePreviewEngine::new("eloquence", "v1", Behavior::Stream);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        engines.register(fallback.clone()).unwrap();
        let mut request = draft();
        request.selection = VoicePreviewSelection::Choice {
            choice_id: "paul-soft".into(),
        };
        let response = run_native(prepare(request, &engines), &engines, false);
        let good = matches!(behavior, Behavior::Buffered | Behavior::Stream);
        assert_eq!(
            response.status,
            if good {
                PreviewStatus::Completed
            } else {
                PreviewStatus::Failed
            }
        );
        if good {
            assert_eq!(
                response.last_started.unwrap().choice_id.as_deref(),
                Some("paul-soft")
            );
        } else {
            assert!(response.accepted_audio.is_empty());
        }
        let calls = engine.parameters.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].native.parameters["sm"],
            omnivox_tts::voice_choices::Adjustment::Set {
                value: omnivox_tts::native_parameters::NativeValue::Integer(80)
            }
        );
        assert!(fallback.inner.requests.lock().unwrap().is_empty());
    }
}
#[test]
fn native_preview_inapplicable_or_invalid_receipts_cannot_play_common_only() {
    for behavior in [Behavior::Buffered, Behavior::Stream] {
        for native in [
            None,
            Some(NativeBehavior::CommonOnly),
            Some(NativeBehavior::Stale),
        ] {
            let mut engine = NativePreviewEngine::new("dectalk", "paul", behavior);
            if let Some(native) = native {
                Arc::get_mut(&mut engine).unwrap().native_behavior = native;
            }
            let mut engines = EngineRegistry::new();
            engines.register(engine.clone()).unwrap();
            let prepared = if native.is_some() {
                prepare(draft(), &engines)
            } else {
                prepare_voice_preview_v3(
                    draft(),
                    0.65,
                    &engines,
                    &RoutingPolicyRegistry::new("dectalk"),
                    vec![],
                )
                .unwrap()
            };
            let response = run_native(prepared, &engines, false);
            assert_eq!(response.status, PreviewStatus::Failed);
            assert!(response.accepted_audio.is_empty());
            assert!(response.last_started.is_none());
            if native.is_none() {
                assert!(response
                    .message
                    .as_deref()
                    .unwrap()
                    .contains("native voice settings unavailable"));
                assert!(engine.inner.requests.lock().unwrap().is_empty());
            }
        }
    }
}
#[test]
fn native_preview_empty_stale_and_post_pcm_failure_keep_truthful_terminals() {
    for (behavior, stale, status) in [
        (Behavior::Empty, false, PreviewStatus::Completed),
        (Behavior::Stream, true, PreviewStatus::Cancelled),
        (Behavior::FailAfterAudio, false, PreviewStatus::Failed),
    ] {
        let engine = NativePreviewEngine::new("dectalk", "paul", behavior);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let response = run_native(prepare(draft(), &engines), &engines, stale);
        assert_eq!(response.status, status);
        if matches!(behavior, Behavior::FailAfterAudio) {
            assert_eq!(response.accepted_audio.len(), 1);
            assert_eq!(engine.inner.requests.lock().unwrap().len(), 1);
        } else {
            assert!(response.accepted_audio.is_empty());
            assert!(response.last_started.is_none());
        }
        if stale {
            assert!(engine.inner.requests.lock().unwrap().is_empty());
        }
    }
}
#[test]
fn native_preview_private_admission_freezes_policy_rate_and_native_payload() {
    let engine = NativePreviewEngine::new("dectalk", "paul", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(engine.clone()).unwrap();
    let mut policy = RoutingPolicyRegistry::new("dectalk");
    policy
        .register(
            1,
            RoutingPolicy {
                disabled_engine_ids: vec!["dectalk".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let mut request = draft();
    request.disabled_engine_ids = vec!["eloquence".into()];
    let prepared = prepare_voice_preview_v3(
        request,
        0.65,
        &engines,
        &policy,
        vec![catalogue("dectalk", "paul")],
    )
    .unwrap();
    assert!(prepared.routing.queued_payload_bytes() > 1024);
    policy.register(2, RoutingPolicy::default()).unwrap();
    let response = run_native(prepared, &engines, false);
    assert_eq!(response.status, PreviewStatus::Failed);
    assert_eq!(
        response.effective_disabled_engine_ids,
        vec!["eloquence", "dectalk"]
    );
    assert_eq!(response.base_rate, 0.65);
    assert!(engine.inner.requests.lock().unwrap().is_empty());
    assert!(prepare_voice_preview_v3(draft(), 0.66, &engines, &policy, vec![]).is_err());
    let mut invalid = draft();
    invalid.voice.choices[0]
        .native
        .as_mut()
        .unwrap()
        .parameters
        .insert(
            "sm".into(),
            omnivox_tts::voice_choices::Adjustment::Set {
                value: omnivox_tts::native_parameters::NativeValue::Integer(101),
            },
        );
    assert!(prepare_voice_preview_v3(
        invalid,
        0.65,
        &engines,
        &policy,
        vec![catalogue("dectalk", "paul")]
    )
    .is_err());
}

#[test]
fn native_preview_without_native_edits_reports_required_null_application() {
    for behavior in [Behavior::Buffered, Behavior::Stream] {
        let engine = NativePreviewEngine::new("dectalk", "paul", behavior);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let mut request = draft();
        request.voice.choices[0].native = None;
        let prepared = prepare_voice_preview_v3(
            request,
            0.65,
            &engines,
            &RoutingPolicyRegistry::new("dectalk"),
            vec![],
        )
        .unwrap();
        let response = run_native(prepared, &engines, false);
        assert_eq!(response.status, PreviewStatus::Completed);
        assert_eq!(response.accepted_audio.len(), 1);
        assert!(response.accepted_audio[0].native_application.is_none());
        assert!(response.last_started.unwrap().native_application.is_none());
        assert!(engine.parameters.lock().unwrap().is_empty());
    }
}
