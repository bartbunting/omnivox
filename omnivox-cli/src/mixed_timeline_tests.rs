//! Ordinary mixed-span execution using the same simulated engines as preview.
use super::*;

#[path = "mixed_timeline_admission_tests.rs"]
mod mixed_timeline_admission_tests;
use omnivox_tts::marker_protocol::{decode_marker_event, MarkerEvent, MarkerEventEnvelope};
use omnivox_tts::timeline_protocol::*;
use omnivox_tts::timeline_v4::{LayeredSpeechSpan, MixedSpeechSpan, PresentationTimelineV4};
use omnivox_tts::voice_choices::{Adjustment, LayeredVoiceDefinition};

fn definition() -> LayeredVoiceDefinition {
    let mut voice = layered_request().voice.definition();
    voice.id = "bolden".to_owned();
    voice.shared.effects.echo = Some(0.25);
    voice
}

fn registry(engines: &EngineRegistry, voice: LayeredVoiceDefinition) -> LogicalVoiceRegistry {
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v2(
            41,
            vec![
                RegisteredVoiceDefinition::Layered(voice),
                RegisteredVoiceDefinition::Legacy(LogicalVoiceDefinition {
                    id: "annotation".to_owned(),
                    language: None,
                    preferences: vec![VoiceSelector::Exact(PhysicalVoiceId::new("second", "two"))],
                    acss: NormalizedAcss::default(),
                    effects: PostSynthesisStyle::default(),
                }),
            ],
            FallbackPolicy::default(),
            &engines.inventory(),
        )
        .unwrap();
    registry
}

fn legacy(id: u64, effects: PresentationEffectDirective) -> MixedSpeechSpan {
    MixedSpeechSpan::Legacy(PresentationSpeechSpan {
        id,
        text: format!("annotation {id}"),
        logical_voice_id: Some("annotation".to_owned()),
        acss: NormalizedAcss {
            richness: Some(0.2),
            ..Default::default()
        },
        rate_offset: None,
        effects,
    })
}

fn layered(id: u64, context: VoiceStylePatch) -> MixedSpeechSpan {
    MixedSpeechSpan::Layered(LayeredSpeechSpan {
        id,
        text: format!("heading {id}"),
        logical_voice_id: "bolden".to_owned(),
        context,
        placement: VoicePlacement::default(),
    })
}

fn timeline() -> PresentationTimelineV4 {
    PresentationTimelineV4 {
        protocol_version: 4,
        generation: 8,
        dispatch_id: 91,
        registry_generation: 41,
        delivery_policy: PresentationDeliveryPolicy::Ordered,
        replacement_key: None,
        spans: vec![
            legacy(
                1,
                PresentationEffectDirective::Replace {
                    state_id: "annotation.effects".to_owned(),
                    style: PostSynthesisStyle {
                        pan: Some(0.2),
                        ..Default::default()
                    },
                },
            ),
            legacy(2, PresentationEffectDirective::Retain),
            layered(
                3,
                VoiceStylePatch {
                    richness: Some(Adjustment::Set { value: 0.5 }),
                    ..Default::default()
                },
            ),
            legacy(4, PresentationEffectDirective::Retain),
            layered(5, VoiceStylePatch::default()),
        ],
        actions: Vec::new(),
    }
}

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

fn run_mixed(
    timeline: PresentationTimelineV4,
    engines: &EngineRegistry,
    routing: LogicalVoiceRoutingSnapshot,
) -> (BatchStatus, Vec<MarkerEventEnvelope>) {
    let streams = AudioStreams::new_with_backend(8, 8, 8, AudioBackend::Null).unwrap();
    let control = streams.control();
    let engine = engines.engine(&engines.inventory()[0].id).unwrap();
    let generation = AtomicU64::new(1);
    let lifecycle = RequestLifecycle::default();
    let tickets = Mutex::new(Vec::new());
    let clock = Mutex::new(Vec::new());
    let overlays = Mutex::new(Vec::new());
    let renderer = Mutex::new(TimelineAudioRenderer::new());
    let effects = Mutex::new(crate::pipeline::DispatchEffects::new());
    let failed = AtomicBool::new(false);
    let capture = Capture::default();
    let (output, reporter) =
        crate::marker_events::spawn_marker_event_reporter_with_writer(capture.clone());
    let dispatch = MarkerDispatchContext::with_voice_choice_events(timeline.dispatch_id, output);
    let ctx = SynthCtx {
        gen: 1,
        gen_counter: &generation,
        cancellation: None,
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
    let status = crate::pipeline::process_presentation_timeline_v4(
        timeline,
        TtsState {
            speech_rate: 0.65,
            current_voice: "one".to_owned(),
            ..Default::default()
        },
        &ctx,
        &AudioFileLoader::with_cache(),
        engines,
        &RuntimeEngineHealth::new(),
        routing,
    );
    let status = await_tracked_playback(status, tickets.into_inner().unwrap());
    control.drain();
    drop(dispatch);
    reporter.join().unwrap();
    let records = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    let events = records
        .lines()
        .map(|line| decode_marker_event(line.split_whitespace().last().unwrap()).unwrap())
        .collect();
    (status, events)
}

#[test]
fn ordinary_mixed_spans_apply_actual_choice_and_end_legacy_effect_runs() {
    for behavior in [Behavior::Buffered, Behavior::Stream] {
        let first = PreviewEngine::new("first", "one", Behavior::FailBeforeAudio);
        let mut second = PreviewEngine::new("second", "two", behavior);
        Arc::get_mut(&mut second)
            .unwrap()
            .descriptor
            .capabilities
            .post_synthesis_dimensions
            .clear();
        let mut engines = EngineRegistry::new();
        engines.register(first.clone()).unwrap();
        engines.register(second.clone()).unwrap();
        let registry = registry(&engines, definition());
        let mut timeline = timeline();
        let MixedSpeechSpan::Layered(span) = &mut timeline.spans[2] else {
            unreachable!()
        };
        span.context.echo = Some(Adjustment::Default {});
        span.placement.pan = Some(0.8);
        timeline.validate_registry(&registry).unwrap();
        crate::pipeline::validate_presentation_timeline_v4_action_windows(
            &timeline,
            &TtsState::default(),
        )
        .unwrap();
        let (status, events) = run_mixed(
            timeline,
            &engines,
            LogicalVoiceRoutingSnapshot::capture(&registry, &engines),
        );
        assert_eq!(status, BatchStatus::Completed);
        let requests = second.requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[2].normalized_acss.average_pitch, Some(0.6));
        assert_eq!(requests[2].normalized_acss.richness, Some(0.5));
        assert_eq!(requests[4].normalized_acss.richness, Some(0.3));
        assert_eq!(requests[3].normalized_acss.richness, Some(0.2));
        let mut receipt_spans = Vec::new();
        let mut pan_utterances = Vec::new();
        let mut echo_utterances = Vec::new();
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.protocol_version, 3);
            match &event.event {
                MarkerEvent::VoiceChoiceApplied(receipt) => {
                    assert!(matches!(
                        events[index - 1].event,
                        MarkerEvent::UtteranceStarted { .. }
                    ));
                    assert_eq!(event.sequence, events[index - 1].sequence + 1);
                    assert_eq!(receipt.choice.choice_id.as_deref(), Some("fallback"));
                    assert_eq!(receipt.registry_generation, 41);
                    receipt_spans.push(receipt.span_id);
                }
                MarkerEvent::TimelineStyleDegraded {
                    utterance_id,
                    degraded_effects,
                    ..
                } => {
                    if degraded_effects.contains(&PostSynthesisDimension::Pan) {
                        pan_utterances.push(*utterance_id);
                    }
                    if degraded_effects.contains(&PostSynthesisDimension::Echo) {
                        echo_utterances.push(*utterance_id);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(receipt_spans, vec![3, 5]);
        assert_eq!(pan_utterances, vec![1, 2, 3]); // Placement applies to 3; legacy retain at 4 is neutral.
        assert_eq!(echo_utterances, vec![5]); // Context default at 3 cannot leak into later spans.
    }
}

#[test]
fn invalid_later_spans_or_windows_reject_before_any_earlier_audio() {
    for case in 0..4 {
        let second = PreviewEngine::new("second", "two", Behavior::Buffered);
        let mut engines = EngineRegistry::new();
        engines.register(second.clone()).unwrap();
        let registry = registry(&engines, definition());
        let mut timeline = timeline();
        match case {
            0 => timeline.registry_generation = 42,
            1 => {
                let MixedSpeechSpan::Layered(span) = &mut timeline.spans[4] else {
                    unreachable!()
                };
                span.logical_voice_id = "missing".to_owned();
            }
            2 => {
                let MixedSpeechSpan::Layered(span) = &mut timeline.spans[4] else {
                    unreachable!()
                };
                span.text = "x".repeat(400_000);
            }
            _ => {
                timeline.actions = (0..513)
                    .map(|index| PresentationTimelineAction {
                        id: format!("cue-{index}"),
                        lifecycle_anchor: PresentationLifecycleAnchor::Object,
                        position: PresentationTimelinePosition::SpanBoundary {
                            span_id: 5,
                            affinity: PresentationAffinity::Before,
                        },
                        action: PresentationAction::SemanticEvent,
                    })
                    .collect()
            }
        }
        if case >= 2 {
            assert!(
                crate::pipeline::validate_presentation_timeline_v4_action_windows(
                    &timeline,
                    &TtsState::default()
                )
                .is_err()
            );
        }
        let (status, events) = run_mixed(
            timeline,
            &engines,
            LogicalVoiceRoutingSnapshot::capture(&registry, &engines),
        );
        assert_eq!(status, BatchStatus::Failed);
        assert!(events.is_empty());
        assert!(second.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn unresolved_layered_span_fails_without_unrelated_legacy_voice() {
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(second.clone()).unwrap();
    let mut voice = definition();
    voice.choices.clear();
    let registry = registry(&engines, voice);
    let mut timeline = timeline();
    timeline.spans = vec![layered(1, VoiceStylePatch::default())];
    timeline.validate_registry(&registry).unwrap();
    let (status, events) = run_mixed(
        timeline,
        &engines,
        LogicalVoiceRoutingSnapshot::capture(&registry, &engines),
    );
    assert_eq!(status, BatchStatus::Failed);
    assert!(events.is_empty());
    assert!(second.requests.lock().unwrap().is_empty());
}

#[test]
fn ordinary_playback_keeps_the_admitted_registry_after_live_replacement() {
    let second = PreviewEngine::new("second", "two", Behavior::Buffered);
    let mut engines = EngineRegistry::new();
    engines.register(second.clone()).unwrap();
    let mut registry = registry(&engines, definition());
    let mut timeline = timeline();
    timeline.spans = vec![layered(1, VoiceStylePatch::default())];
    timeline.validate_registry(&registry).unwrap();
    let snapshot = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
    let mut replacement = definition();
    replacement.choices[1].adjustments.richness = Some(Adjustment::Set { value: 0.9 });
    registry
        .register_v2(
            42,
            vec![RegisteredVoiceDefinition::Layered(replacement)],
            FallbackPolicy::default(),
            &engines.inventory(),
        )
        .unwrap();
    let (status, events) = run_mixed(timeline, &engines, snapshot);
    assert_eq!(status, BatchStatus::Completed);
    assert_eq!(
        second.requests.lock().unwrap()[0].normalized_acss.richness,
        Some(0.3)
    );
    assert!(events.iter().any(|event| matches!(&event.event, MarkerEvent::VoiceChoiceApplied(receipt) if receipt.registry_generation == 41)));
    assert_eq!(registry.generation(), 42);
}

#[test]
fn a_legacy_run_starts_neutral_even_when_its_registered_voice_has_effects() {
    let mut second = PreviewEngine::new("second", "two", Behavior::Buffered);
    Arc::get_mut(&mut second)
        .unwrap()
        .descriptor
        .capabilities
        .post_synthesis_dimensions
        .clear();
    let mut engines = EngineRegistry::new();
    engines.register(second.clone()).unwrap();
    let original = registry(&engines, definition());
    let mut definitions = original.registered_definitions().to_vec();
    let RegisteredVoiceDefinition::Legacy(voice) = &mut definitions[1] else {
        unreachable!()
    };
    voice.effects.pan = Some(0.9);
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v2(
            41,
            definitions,
            FallbackPolicy::default(),
            &engines.inventory(),
        )
        .unwrap();
    let mut timeline = timeline();
    timeline.spans = vec![legacy(1, PresentationEffectDirective::Retain)];
    let (status, events) = run_mixed(
        timeline,
        &engines,
        LogicalVoiceRoutingSnapshot::capture(&registry, &engines),
    );
    assert_eq!(status, BatchStatus::Completed);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        MarkerEvent::UtteranceStarted { .. }
    ));
}
