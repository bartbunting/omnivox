mod native_timeline_playback {
    use super::*;
    use crate::lifecycle::RequestLifecycle;
    use crate::marker_events::{spawn_marker_event_reporter_with_writer, MarkerDispatchContext};
    use crate::native_plans::NativePlanReferences;
    use crate::pipeline::{BatchStatus, SynthCtx};
    use omnivox_audio::{
        AudioBackend, AudioFileLoader, AudioStreams, PlaybackStatus, TimelineAudioRenderer,
    };
    use omnivox_core::TtsState;
    use omnivox_tts::marker_protocol::{decode_marker_event, MarkerEvent, MarkerEventEnvelope};
    use omnivox_tts::timeline_v5::PresentationTimelineV5;
    use std::sync::atomic::AtomicBool;
    #[derive(Clone, Default)]
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
    fn timeline() -> PresentationTimelineV5 {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/engine-voice-parameters.json"
        ))
        .unwrap();
        serde_json::from_value(fixture["messages"]["timeline"].clone()).unwrap()
    }
    fn run(
        engines: &EngineRegistry,
        routing: LogicalVoiceRoutingSnapshot,
    ) -> (
        BatchStatus,
        Vec<MarkerEventEnvelope>,
        Arc<NativePlanReferences>,
    ) {
        let streams = AudioStreams::new_with_backend(8, 8, 8, AudioBackend::Null).unwrap();
        let control = streams.control();
        let engine = engines.engine(&engines.inventory()[0].id).unwrap();
        let generation = AtomicU64::new(1);
        let cancellation = SynthesisCancellationToken::new();
        let lifecycle = RequestLifecycle::default();
        let tickets = Mutex::new(vec![]);
        let clock = Mutex::new(vec![]);
        let overlays = Mutex::new(vec![]);
        let renderer = Mutex::new(TimelineAudioRenderer::new());
        let effects = Mutex::new(crate::pipeline::DispatchEffects::new());
        let failed = AtomicBool::new(false);
        let capture = Capture::default();
        let (output, reporter) = spawn_marker_event_reporter_with_writer(capture.clone());
        let plans = Arc::new(NativePlanReferences::default());
        let dispatch = MarkerDispatchContext::with_native_events(91, output, plans.clone());
        let ctx = SynthCtx {
            letter_navigation: false,
            gen: 1,
            gen_counter: &generation,
            cancellation: Some(&cancellation),
            lifecycle: &lifecycle,
            engine: engine.as_ref(),
            control: &control,
            playback_tickets: Some(&tickets),
            presentation_clock: Some(&clock),
            pending_overlays: Some(&overlays),
            timeline_renderer: Some(&renderer),
            effect_processor: Some(&effects),
            marker_span_id: None,
            marker_dispatch: Some(&dispatch),
            voice_observations: None,
            batch_failed: Some(&failed),
        };
        let mut status = crate::pipeline::process_presentation_timeline_v4(
            timeline().into_execution(),
            TtsState {
                speech_rate: 0.65,
                current_voice: "Paul".into(),
                ..Default::default()
            },
            &ctx,
            &AudioFileLoader::with_cache(),
            engines,
            &RuntimeEngineHealth::new(),
            routing,
        );
        for ticket in tickets.into_inner().unwrap() {
            if ticket.wait() == PlaybackStatus::Cancelled && status == BatchStatus::Completed {
                status = BatchStatus::Cancelled;
            }
        }
        control.drain();
        drop(dispatch);
        reporter.join().unwrap();
        let records = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let events = records
            .lines()
            .map(|line| decode_marker_event(line.split_whitespace().last().unwrap()).unwrap())
            .collect();
        (status, events, plans)
    }
    #[test]
    fn native_timeline_fallback_reports_consumed_choice_for_every_playback_mode() {
        for first_stream in [false, true] {
            for fallback_stream in [false, true] {
                let first = NativeEngine::new(
                    "dectalk",
                    "Paul",
                    first_stream,
                    Some(if first_stream {
                        MockFailure::StreamBeforeAudio
                    } else {
                        MockFailure::Synthesis
                    }),
                    ReceiptBehavior::Good,
                );
                let second = NativeEngine::new(
                    "eloquence",
                    "Reed",
                    fallback_stream,
                    None,
                    ReceiptBehavior::Good,
                );
                let mut engines = EngineRegistry::new();
                engines.register(first).unwrap();
                engines.register(second.clone()).unwrap();
                let base = snapshot(&engines, native_definition());
                let bytes = base.queued_payload_bytes();
                let routing = base.with_parameter_catalogues(vec![
                    Arc::new(metadata("dectalk", "Paul")),
                    Arc::new(metadata("eloquence", "Reed")),
                ]);
                assert!(routing.queued_payload_bytes() > bytes);
                let (status, events, plans) = run(&engines, routing);
                assert_eq!(status, BatchStatus::Completed);
                let mut ids = vec![];
                for (i, event) in events.iter().enumerate() {
                    assert_eq!(event.protocol_version, 4);
                    if let MarkerEvent::VoiceChoiceApplied(choice) = &event.event {
                        assert!(matches!(
                            events[i - 1].event,
                            MarkerEvent::UtteranceStarted { .. }
                        ));
                        assert_eq!(
                            choice.choice.realized,
                            PhysicalVoiceId::new("eloquence", "Reed")
                        );
                        let application = choice
                            .native_application
                            .as_ref()
                            .unwrap()
                            .as_ref()
                            .unwrap();
                        assert_eq!(application.status, ApplicationStatus::Applied);
                        let id = application.plan_id.as_ref().unwrap();
                        assert_ne!(id, "eloquence-plan");
                        ids.push(id.clone());
                        let reference = plans.lookup(id, &engines).unwrap();
                        assert_eq!(reference.helper_id, "eloquence-plan");
                        assert_eq!(&reference.identity, application.identity.as_ref().unwrap());
                    }
                }
                assert_eq!(ids.len(), 2);
                assert_ne!(ids[0], ids[1]);
                let calls = second.native_calls.lock().unwrap();
                assert!(calls[0]
                    .1
                    .context_dimensions
                    .contains(&CommonInput::Richness));
                assert!(!calls[1]
                    .1
                    .context_dimensions
                    .contains(&CommonInput::Richness));
                assert_eq!(calls[0].1.native.parameters, calls[1].1.native.parameters);
            }
        }
    }
    #[test]
    fn native_timeline_missing_catalogue_explicitly_reports_common_only() {
        for stream in [false, true] {
            let engine = NativeEngine::new("dectalk", "Paul", stream, None, ReceiptBehavior::Good);
            let mut engines = EngineRegistry::new();
            engines.register(engine.clone()).unwrap();
            let (status, events, _) = run(&engines, snapshot(&engines, native_definition()));
            assert_eq!(status, BatchStatus::Completed);
            let choices = events
                .iter()
                .filter_map(|e| {
                    if let MarkerEvent::VoiceChoiceApplied(c) = &e.event {
                        Some(c)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(choices.len(), 2);
            for choice in choices {
                let application = choice
                    .native_application
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap();
                assert_eq!(application.status, ApplicationStatus::CommonOnly);
                assert!(application.plan_id.is_none());
            }
            assert!(engine.native_calls.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn native_timeline_unconsumed_audio_never_claims_applied() {
        for behavior in [
            ReceiptBehavior::Missing,
            ReceiptBehavior::Duplicate,
            ReceiptBehavior::Stale,
            ReceiptBehavior::Cancel,
            ReceiptBehavior::Empty,
            ReceiptBehavior::EmptyWithoutStart,
        ] {
            let engine = NativeEngine::new("dectalk", "Paul", true, None, behavior);
            let mut engines = EngineRegistry::new();
            engines.register(engine).unwrap();
            let routing = snapshot(&engines, native_definition())
                .with_parameter_catalogues(vec![Arc::new(metadata("dectalk", "Paul"))]);
            let (_, events, _) = run(&engines, routing);
            assert!(!events.iter().any(|e| matches!(
                e.event,
                MarkerEvent::UtteranceStarted { .. } | MarkerEvent::VoiceChoiceApplied(_)
            )));
        }
    }
    #[test]
    fn applied_plan_references_expire_on_eviction_or_runtime_replacement() {
        let engine = NativeEngine::new("dectalk", "Paul", false, None, ReceiptBehavior::Good);
        let mut engines = EngineRegistry::new();
        engines.register(engine.clone()).unwrap();
        let owner: Arc<dyn TtsEngine> = engine.clone();
        let weak = Arc::downgrade(&owner);
        let runtime = Some((weak.clone(), 1));
        let voice = PhysicalVoiceId::new("dectalk", "Paul");
        let application = NativeApplication {
            status: ApplicationStatus::Applied,
            plan_id: Some("same-helper-id".into()),
            identity: Some(metadata("dectalk", "Paul").identity),
            masked_parameters: vec![],
            reason: None,
        };
        let plans = NativePlanReferences::default();
        let first = plans
            .publish(&runtime, &voice, &application, Some("choice"))
            .plan_id
            .unwrap();
        assert!(plans.lookup(&first, &engines).is_some());
        for _ in 0..64 {
            plans.publish(&runtime, &voice, &application, Some("choice"));
        }
        assert!(plans.lookup(&first, &engines).is_none());
        let last = plans
            .publish(&runtime, &voice, &application, Some("choice"))
            .plan_id
            .unwrap();
        assert!(plans.lookup(&last, &engines).is_some());
        engine.epoch.store(2, Ordering::Release);
        assert!(plans.lookup(&last, &engines).is_none());
        // Publishing an old receipt after restart must not rebind it to epoch 2.
        let stale = plans
            .publish(&runtime, &voice, &application, Some("choice"))
            .plan_id
            .unwrap();
        assert!(plans.lookup(&stale, &engines).is_none());
        let other = NativePlanReferences::default();
        assert!(other.lookup(&last, &engines).is_none());
        drop(owner);
        drop(engine);
        drop(engines);
        assert_eq!(weak.strong_count(), 0);
    }
}
