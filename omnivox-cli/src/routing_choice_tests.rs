use super::*;
use crate::routing::choice::{
    AttemptStyle, PreparedSynthesisOutcome, PreparedVoiceAttempt, RoutedPlaybackSink,
};
use omnivox_tts::resolver::ResolutionReason;
use omnivox_tts::voice_choices::{Adjustment, VoiceStylePatch};

fn fixture_voice() -> LayeredVoiceDefinition {
    // Independent contract example: docs/protocol-fixtures/voice-choice-tuning.json.
    use omnivox_tts::voice_choices::{SharedAcss, SharedEffects, SharedVoiceStyle, VoiceChoice};
    LayeredVoiceDefinition {
        id: "bolden".to_owned(),
        language: Some("en-AU".to_owned()),
        shared: SharedVoiceStyle {
            acss: SharedAcss {
                rate: None,
                average_pitch: Some(0.4),
                pitch_range: None,
                stress: None,
                richness: Some(0.5),
                volume: None,
            },
            rate_offset: Some(2),
            effects: SharedEffects {
                gain: None,
                low_pass: Some(0.75),
                high_pass: None,
                pan: None,
                reverb: None,
                echo: None,
                chorus: None,
            },
        },
        choices: vec![
            VoiceChoice {
                id: "dectalk-paul".to_owned(),
                selector: exact("dectalk", "Paul"),
                adjustments: VoiceStylePatch {
                    average_pitch: Some(Adjustment::Set { value: 0.2 }),
                    richness: Some(Adjustment::Set { value: 0.8 }),
                    rate_offset: Some(Adjustment::Set { value: 4 }),
                    low_pass: Some(Adjustment::Default {}),
                    ..Default::default()
                },
            },
            VoiceChoice {
                id: "eloquence-reed".to_owned(),
                selector: exact("eloquence", "Reed"),
                adjustments: VoiceStylePatch {
                    average_pitch: Some(Adjustment::Set { value: 0.6 }),
                    richness: Some(Adjustment::Set { value: 0.3 }),
                    rate_offset: Some(Adjustment::Set { value: -1 }),
                    low_pass: Some(Adjustment::Set { value: 0.33 }),
                    ..Default::default()
                },
            },
            VoiceChoice {
                id: "eloquence-reed-soft".to_owned(),
                selector: exact("eloquence", "Reed"),
                adjustments: VoiceStylePatch {
                    richness: Some(Adjustment::Set { value: 0.2 }),
                    gain: Some(Adjustment::Set { value: 0.4 }),
                    ..Default::default()
                },
            },
        ],
    }
}

fn full_engine(
    id: &str,
    voice: &str,
    streaming: bool,
    failure: Option<MockFailure>,
) -> Arc<MockEngine> {
    let mut engine = synthesis_engine(id, voice, failure);
    let capabilities = &mut Arc::get_mut(&mut engine).unwrap().descriptor.capabilities;
    capabilities.audio_output = if streaming {
        AudioOutputMode::StreamingPcm
    } else {
        AudioOutputMode::BufferedPcm
    };
    capabilities.acss = AcssCapabilities {
        rate: true,
        average_pitch: true,
        pitch_range: true,
        stress: true,
        richness: true,
        volume: true,
    };
    engine
}

fn registered(
    engines: &EngineRegistry,
    definition: LayeredVoiceDefinition,
) -> LogicalVoiceRegistry {
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v2(
            41,
            vec![RegisteredVoiceDefinition::Layered(definition)],
            FallbackPolicy::default(),
            &engines.inventory(),
        )
        .unwrap();
    registry
}

#[derive(Default)]
struct AttemptSink {
    attempts: Vec<PreparedVoiceAttempt>,
    stream: RecordingStreamSink,
    fail_start: bool,
}

impl RoutedPlaybackSink for AttemptSink {
    fn start_attempt(
        &mut self,
        attempt: &PreparedVoiceAttempt,
        start: SynthesisStreamStart,
    ) -> Result<(), TtsError> {
        assert_eq!(
            start.actual_voice.as_ref(),
            Some(&attempt.resolution.realized)
        );
        self.attempts.push(attempt.clone());
        if self.fail_start {
            return Err(TtsError::SynthesisFailed("output unavailable".to_owned()));
        }
        self.stream.start(start)
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.stream.audio(audio)
    }
    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        self.stream.markers(markers, anchors)
    }
}

fn run(
    engines: &EngineRegistry,
    routing: &mut LogicalVoiceRoutingSnapshot,
    style: &AttemptStyle<'_>,
    sink: &mut AttemptSink,
) -> PreparedSynthesisOutcome {
    let mut route = routing.initial_route("bolden", engines).unwrap();
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
        None,
        sink,
    )
}

#[test]
fn all_buffered_and_streaming_retry_combinations_handoff_the_actual_choice() {
    for primary_streaming in [false, true] {
        for fallback_streaming in [false, true] {
            let primary = full_engine(
                "dectalk",
                "Paul",
                primary_streaming,
                Some(if primary_streaming {
                    MockFailure::StreamAfterMarkers
                } else {
                    MockFailure::Synthesis
                }),
            );
            let fallback = full_engine("eloquence", "Reed", fallback_streaming, None);
            let mut engines = EngineRegistry::new();
            engines.register(primary.clone()).unwrap();
            engines.register(fallback.clone()).unwrap();
            let registry = registered(&engines, fixture_voice());
            let mut routing = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
            let context = VoiceStylePatch {
                stress: Some(Adjustment::Set { value: 0.7 }),
                ..Default::default()
            };
            let style = AttemptStyle::Layered {
                context: &context,
                base_rate: 0.5,
                placement_pan: Some(0.8),
            };
            let mut sink = AttemptSink::default();
            let outcome = run(&engines, &mut routing, &style, &mut sink);
            let prepared = match outcome {
                PreparedSynthesisOutcome::Buffered { result, attempt } => {
                    assert!(!fallback_streaming);
                    assert_eq!(
                        result.actual_voice,
                        Some(PhysicalVoiceId::new("eloquence", "Reed"))
                    );
                    assert!(sink.attempts.is_empty());
                    *attempt
                }
                PreparedSynthesisOutcome::Streamed(_) => {
                    assert!(fallback_streaming);
                    assert_eq!(sink.attempts.len(), 1);
                    assert_eq!(sink.stream.audio.len(), 1);
                    assert!(sink.stream.anchors.is_empty());
                    sink.attempts.pop().unwrap()
                }
                _ => panic!("expected successful fallback"),
            };
            assert_eq!(prepared.registry_generation, 41);
            assert_eq!(prepared.choice_index, Some(1));
            assert_eq!(prepared.choice_id.as_deref(), Some("eloquence-reed"));
            assert_eq!(
                prepared.resolution.reason,
                ResolutionReason::ExplicitAlternative {
                    preference_index: 1
                }
            );
            assert_eq!(prepared.acss.style.richness, Some(0.3));
            assert_eq!(prepared.acss.style.stress, Some(0.7));
            assert_eq!(prepared.effects.style.low_pass, Some(0.33));
            assert_eq!(prepared.effects.style.pan, Some(0.8));
            let settings = fallback.settings.lock().unwrap();
            assert_eq!(settings.len(), 1);
            assert!((settings[0].rate - 0.49).abs() < 0.00001);
            assert!((settings[0].pitch - 1.08).abs() < 0.00001);
            assert_eq!(settings[0].volume, 1.0);
            assert!((primary.settings.lock().unwrap()[0].rate - 0.54).abs() < 0.00001);
        }
    }
}

#[test]
fn admitted_definition_is_frozen_and_default_is_fresh_across_chunks() {
    let engine = full_engine("dectalk", "Paul", false, None);
    let mut engines = EngineRegistry::new();
    engines.register(engine.clone()).unwrap();
    let mut registry = registered(&engines, fixture_voice());
    let mut routing = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
    let mut replacement = fixture_voice();
    replacement.choices[0].adjustments.average_pitch = Some(Adjustment::Set { value: 0.9 });
    registry
        .register_v2(
            42,
            vec![RegisteredVoiceDefinition::Layered(replacement)],
            FallbackPolicy::default(),
            &engines.inventory(),
        )
        .unwrap();
    let custom = VoiceStylePatch::default();
    let reset = VoiceStylePatch {
        average_pitch: Some(Adjustment::Default {}),
        richness: Some(Adjustment::Default {}),
        rate_offset: Some(Adjustment::Default {}),
        low_pass: Some(Adjustment::Default {}),
        ..Default::default()
    };
    for context in [&custom, &reset, &custom] {
        let style = AttemptStyle::Layered {
            context,
            base_rate: 1.5,
            placement_pan: None,
        };
        let PreparedSynthesisOutcome::Buffered { attempt, .. } =
            run(&engines, &mut routing, &style, &mut AttemptSink::default())
        else {
            panic!("buffered result expected")
        };
        assert_eq!(attempt.registry_generation, 41);
        assert_eq!(attempt.effects.style.low_pass, None);
    }
    let settings = engine.settings.lock().unwrap();
    assert_eq!(settings.len(), 3);
    assert!((settings[0].pitch - 0.68).abs() < 0.00001);
    assert_eq!(settings[1].pitch, 1.0);
    assert_eq!(settings[1].rate, 1.5);
    assert_eq!(settings[2].pitch, settings[0].pitch);
    assert_eq!(engine.normalized_acss.lock().unwrap()[1].richness, None);
    // Every independently queued snapshot includes its retained authoritative rows.
    let legacy = snapshot(
        &engines,
        fixture_voice().legacy_projection(),
        FallbackPolicy::default(),
    );
    assert!(
        routing.queued_payload_bytes()
            >= legacy.queued_payload_bytes() + std::mem::size_of::<LayeredVoiceDefinition>()
    );
}

#[test]
fn duplicate_selectors_use_original_index_but_policy_fallback_has_no_patch() {
    let engine = full_engine("eloquence", "Reed", false, None);
    let mut engines = EngineRegistry::new();
    engines.register(engine).unwrap();
    let registry = registered(&engines, fixture_voice());
    let routing = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
    let mut route = routing.initial_route("bolden", &engines).unwrap();
    let context = VoiceStylePatch::default();
    let style = AttemptStyle::Layered {
        context: &context,
        base_rate: 0.5,
        placement_pan: None,
    };
    // Strict preview will restore this original index after its private one-row resolver.
    route.resolution.reason = ResolutionReason::ExplicitAlternative {
        preference_index: 2,
    };
    let prepared = style
        .prepare(&routing, &route, &route.engine.descriptor())
        .unwrap();
    assert_eq!(prepared.choice_id.as_deref(), Some("eloquence-reed-soft"));
    assert_eq!(prepared.choice_index, Some(2));
    route.resolution.reason = ResolutionReason::GlobalDefault;
    let policy = style
        .prepare(&routing, &route, &route.engine.descriptor())
        .unwrap();
    assert_eq!(policy.choice_id, None);
    assert_eq!(policy.choice_index, None);
    assert_eq!(policy.acss.style.richness, Some(0.5));
    assert_eq!(policy.effects.style.low_pass, Some(0.75));
}

#[test]
fn stream_identity_must_match_before_any_preamble_is_committed() {
    for failure in [
        MockFailure::StreamWrongVoice,
        MockFailure::StreamMissingVoice,
    ] {
        let primary = full_engine("dectalk", "Paul", true, Some(failure));
        let fallback = full_engine("eloquence", "Reed", true, None);
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(fallback).unwrap();
        let registry = registered(&engines, fixture_voice());
        let mut routing = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
        let context = VoiceStylePatch::default();
        let style = AttemptStyle::Layered {
            context: &context,
            base_rate: 0.5,
            placement_pan: None,
        };
        let mut sink = AttemptSink::default();
        assert!(matches!(
            run(&engines, &mut routing, &style, &mut sink),
            PreparedSynthesisOutcome::Streamed(_)
        ));
        assert_eq!(sink.attempts.len(), 1);
        assert_eq!(
            sink.attempts[0].choice_id.as_deref(),
            Some("eloquence-reed")
        );
    }
}

#[test]
fn committed_audio_or_output_failure_never_retries_a_choice() {
    for output_error in [false, true] {
        let primary = full_engine("dectalk", "Paul", true, Some(MockFailure::StreamAfterAudio));
        let fallback = full_engine("eloquence", "Reed", true, None);
        let mut engines = EngineRegistry::new();
        engines.register(primary).unwrap();
        engines.register(fallback.clone()).unwrap();
        let registry = registered(&engines, fixture_voice());
        let mut routing = LogicalVoiceRoutingSnapshot::capture(&registry, &engines);
        let context = VoiceStylePatch::default();
        let style = AttemptStyle::Layered {
            context: &context,
            base_rate: 0.5,
            placement_pan: None,
        };
        let mut sink = AttemptSink {
            fail_start: output_error,
            ..Default::default()
        };
        assert!(matches!(
            run(&engines, &mut routing, &style, &mut sink),
            PreparedSynthesisOutcome::Failed
        ));
        assert_eq!(sink.attempts.len(), 1);
        assert!(fallback.calls.lock().unwrap().is_empty());
        assert_eq!(sink.stream.audio.is_empty(), output_error);
    }
}
