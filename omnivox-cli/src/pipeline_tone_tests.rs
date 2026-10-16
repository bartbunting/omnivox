use super::*;
use std::cell::Cell;

#[test]
fn standalone_and_timeline_share_shape_gain_and_routing_for_insert_and_overlay() {
    use omnivox_core::tone::{AdsrEnvelope, Waveform};
    for waveform in [Waveform::Sine, Waveform::Triangle, Waveform::Square, Waveform::Pulse, Waveform::Sawtooth] {
        for envelope in [
            None,
            Some(AdsrEnvelope {
                attack_ms: 1,
                decay_ms: 25,
                sustain_level: 0.25,
                release_ms: 10,
            }),
        ] {
            for mode in [AudioActionMode::Insert, AudioActionMode::Overlay] {
                for channel in [ChannelMode::Both, ChannelMode::Left, ChannelMode::Right] {
                    let mut state = TtsState {
                        tone_volume: 0.4,
                        ..Default::default()
                    };
                    state.tone_routing.channel_mode = channel;
                    let tone = ToneParameters {
                        frequency_hz: 880.0,
                        duration_ms: 40,
                        volume: 0.35,
                        shape: ToneShape { waveform, envelope,
                            pulse_width: (waveform == Waveform::Pulse).then_some(0.25),
                            end_frequency_hz: Some(1320.0),
                            envelope_curve: envelope.map(|_| omnivox_core::tone::EnvelopeCurve::Exponential),
                        },
                    };
                    let standalone =
                        prepare_shaped_tone_audio(&tone, &state, &never_cancelled).unwrap();
                    let base = boundary_action(
                        "cue",
                        PresentationAction::Tone {
                            frequency_hz: tone.frequency_hz,
                            duration_ms: tone.duration_ms,
                            mode: if mode == AudioActionMode::Insert {
                                PresentationAudioMode::Insert
                            } else {
                                PresentationAudioMode::Overlay
                            },
                            volume: tone.volume,
                            pan: 0.5,
                            effect_bus: PresentationEffectBus::Dry,
                        },
                    );
                    let mut resources = prepare_timeline_resources(
                        &[base],
                        &state,
                        &AudioFileLoader::new(),
                        &never_cancelled,
                    )
                    .unwrap();
                    apply_tone_shapes(&mut resources, &HashMap::from([("cue".into(), tone.shape)]))
                        .unwrap();
                    let action = TimelineChunkAction {
                        id: "cue".into(),
                        text_offset: 0,
                        affinity: AnchorAffinity::Before,
                        kind: TimelineChunkActionKind::Audio {
                            resource: resources.remove("cue").unwrap(),
                            mode,
                            volume: tone.volume,
                            effect_bus: EffectBus::Dry,
                        },
                    };
                    let primary = if mode == AudioActionMode::Insert {
                        AudioBuffer::empty()
                    } else {
                        AudioBuffer::new(vec![0.0; standalone.samples.len()])
                    };
                    let result = CanonicalSynthesisResult {
                        audio: primary,
                        engine_id: "mock".into(),
                        actual_voice: None,
                        markers: Vec::new(),
                        anchors: vec![ResolvedAnchor {
                            id: "cue".into(),
                            frame_offset: Some(0),
                            resolution: AnchorResolution::Exact,
                        }],
                        degraded_acss: Vec::new(),
                    };
                    let (timeline, resources) = prepare_speech_timeline(
                        &result,
                        &[],
                        &[action],
                        &PostSynthesisStyle::default(),
                        &state,
                        &never_cancelled,
                    )
                    .unwrap();
                    let rendered = TimelineAudioRenderer::new()
                        .render_shared_window(&result.audio, &timeline, &resources, true)
                        .unwrap();
                    assert_eq!(rendered.audio.frame_count(), standalone.frame_count());
                    for (actual, expected) in rendered.audio.samples.iter().zip(&standalone.samples)
                    {
                        assert!(
                            (actual - expected).abs() < 0.000001,
                            "gain or shape changed: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}

fn tone_action(duration_ms: u32) -> TimelineChunkAction {
    TimelineChunkAction {
        id: "cue".into(),
        text_offset: 0,
        affinity: AnchorAffinity::Before,
        kind: TimelineChunkActionKind::Audio {
            resource: TimelineAudioResource::Tone {
                frequency_hz: 440.0,
                duration_ms,
                pan: 0.5,
                shape: ToneShape {
                    waveform: omnivox_core::tone::Waveform::Triangle,
                    envelope: Some(omnivox_core::tone::AdsrEnvelope {
                        attack_ms: 10,
                        decay_ms: 25,
                        sustain_level: 0.25,
                        release_ms: 10,
                    }),
                    ..ToneShape::default()
                },
            },
            mode: AudioActionMode::Overlay,
            volume: 0.35,
            effect_bus: EffectBus::Dry,
        },
    }
}

#[test]
fn materialization_and_capital_cues_stop_during_generation_for_both_cancellation_sources() {
    for keyed in [false, true] {
        for capital in [false, true] {
            let generation = AtomicU64::new(1);
            let token = SynthesisCancellationToken::new();
            let checks = Cell::new(0);
            let cancelled = || {
                checks.set(checks.get() + 1);
                if checks.get() == 6 {
                    if keyed {
                        token.cancel();
                    } else {
                        generation.store(2, Ordering::Release);
                    }
                }
                is_stale(1, &generation) || token.is_cancelled()
            };
            let tones = if capital {
                vec![CapitalizationTone {
                    id: "capital".into(),
                    text_offset: 0,
                    frequency_hz: 440.0,
                    duration_ms: 60000,
                }]
            } else {
                Vec::new()
            };
            let actions = if capital {
                Vec::new()
            } else {
                vec![tone_action(60000)]
            };
            let error = prepare_speech_timeline_actions(
                &tones,
                &actions,
                &PostSynthesisStyle::default(),
                &TtsState::default(),
                &cancelled,
            )
            .unwrap_err();
            assert!(matches!(error, omnivox_audio::AudioError::Cancelled));
            // This point is inside generation (after at least 256 frames),
            // not a preflight or the end of the requested sixty-second cue.
            assert_eq!(checks.get(), 6);
        }
    }
}

#[test]
fn standalone_preparation_discards_an_interrupted_tone() {
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() == 2
    };
    assert!(matches!(
        prepare_tone_audio(440.0, 60000, &TtsState::default(), &cancelled),
        Err(omnivox_audio::AudioError::Cancelled)
    ));
    assert_eq!(checks.get(), 2);
}

#[test]
fn cancelled_preparation_never_marks_failure_or_queues_dry_speech() {
    for keyed in [false, true] {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let generation = AtomicU64::new(1);
        let token = SynthesisCancellationToken::new();
        let lifecycle = RequestLifecycle::default();
        let engine = PipelineTestEngine;
        let tickets = Mutex::new(Vec::new());
        let overlays = Mutex::new(Vec::new());
        let effects = Mutex::new(DispatchEffects::new());
        let renderer = Mutex::new(TimelineAudioRenderer::new());
        let failed = AtomicBool::new(false);
        let state = TtsState::default();
        let ctx = SynthCtx {
            letter_navigation: false,
            gen: 1,
            gen_counter: &generation,
            cancellation: Some(&token),
            lifecycle: &lifecycle,
            engine: &engine,
            control: &control,
            playback_tickets: Some(&tickets),
            presentation_clock: None,
            pending_overlays: Some(&overlays),
            timeline_renderer: Some(&renderer),
            effect_processor: Some(&effects),
            marker_span_id: None,
            marker_dispatch: None,
            voice_observations: None,
            batch_failed: Some(&failed),
        };
        let actions = [tone_action(10)];
        let attempt = prepared_attempt(PostSynthesisStyle {
            echo: Some(0.7),
            ..Default::default()
        });
        // An actual voice attempt can be cancelled after prediction but before
        // its action resources are rebuilt for the committed effects.
        let mut sink = ProgressiveChunkSink::new(
            "test",
            None,
            PostSynthesisStyle::default(),
            Vec::new(),
            &state,
            true,
            true,
            &[],
            &actions,
            &ctx,
        )
        .unwrap();
        if keyed {
            token.cancel();
        } else {
            generation.store(2, Ordering::Release);
        }
        assert!(crate::routing::choice::RoutedPlaybackSink::start_attempt(
            &mut sink,
            &attempt,
            SynthesisStreamStart {
                engine_id: "mock".into(),
                actual_voice: Some(attempt.resolution.realized.clone()),
                degraded_acss: Vec::new(),
            },
        )
        .is_err());
        assert!(sink.start.is_none());
        assert!(sink.attempt.is_none());
        drop(sink);
        // Both ordinary buffered speech and prepared buffered speech must stop.
        for prepared in [None, Some(&attempt)] {
            assert!(!queue_synthesis_result_inner(
                result(AudioBuffer::new(vec![0.2; 4000])),
                "test",
                None,
                &[],
                &actions,
                &PostSynthesisStyle::default(),
                &[],
                &state,
                true,
                true,
                prepared,
                &ctx,
            ));
        }
        assert!(matches!(
            ProgressiveChunkSink::new(
                "test",
                None,
                PostSynthesisStyle::default(),
                Vec::new(),
                &state,
                true,
                true,
                &[],
                &actions,
                &ctx,
            ),
            Err(omnivox_audio::AudioError::Cancelled)
        ));
        assert!(matches!(
            synthesize_direct_chunk_progressively(
                &SynthesisRequest::new("test", TtsSettings::default()),
                "test",
                None,
                &[],
                &actions,
                PostSynthesisStyle::default(),
                Vec::new(),
                &state,
                true,
                true,
                &ctx,
            ),
            Err(ProgressiveChunkError::Cancelled)
        ));
        assert!(!failed.load(Ordering::Acquire));
        assert!(tickets.lock().unwrap().is_empty());
        assert!(overlays.lock().unwrap().is_empty());
        assert!(effects.lock().unwrap().finish().is_none());
    }
}
