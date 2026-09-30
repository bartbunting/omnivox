use super::*;
use omnivox_audio::{AudioBackend, AudioStreams};
use omnivox_core::settings::{CapitalPitch, CapitalPitchSettings};
use omnivox_core::state::TtsState;

#[test]
fn letter_cue_follows_actual_engine_through_buffered_and_streaming_fallback() {
    for primary_streams in [false, true] {
        for fallback_streams in [false, true] {
            for (text, global, primary_cue, fallback_cue, first_pitch, last_pitch) in [
                ("A", CapitalPitch::Value(1.5), None, None, 1.5, 1.5),
                (
                    "Č",
                    CapitalPitch::Value(1.4),
                    Some(CapitalPitch::Value(1.8)),
                    None,
                    1.8,
                    1.4,
                ),
                (
                    "A",
                    CapitalPitch::Off,
                    Some(CapitalPitch::Value(1.7)),
                    None,
                    1.7,
                    0.8,
                ),
                (
                    "A",
                    CapitalPitch::Value(1.4),
                    None,
                    Some(CapitalPitch::Off),
                    1.4,
                    0.8,
                ),
                (
                    "A",
                    CapitalPitch::Off,
                    None,
                    Some(CapitalPitch::Value(1.6)),
                    0.8,
                    1.6,
                ),
                (
                    "a",
                    CapitalPitch::Value(1.5),
                    Some(CapitalPitch::Value(1.8)),
                    None,
                    0.8,
                    0.8,
                ),
                ("7", CapitalPitch::Value(1.5), None, None, 0.8, 0.8),
            ] {
                let primary = if primary_streams {
                    streaming_synthesis_engine(
                        "dectalk",
                        "paul",
                        Some(MockFailure::StreamBeforeAudio),
                    )
                } else {
                    synthesis_engine("dectalk", "paul", Some(MockFailure::Synthesis))
                };
                let fallback = if fallback_streams {
                    streaming_synthesis_engine("espeak", "en-us", None)
                } else {
                    synthesis_engine("espeak", "en-us", None)
                };
                let mut engines = EngineRegistry::new();
                engines.register(primary.clone()).unwrap();
                engines.register(fallback.clone()).unwrap();
                let routes = snapshot(
                    &engines,
                    definition(Vec::new()),
                    FallbackPolicy {
                        fallback_engines: vec!["espeak".into()],
                        ..FallbackPolicy::default()
                    },
                );
                let mut capitals = CapitalPitchSettings {
                    default: global,
                    engines: Default::default(),
                };
                if let Some(cue) = primary_cue {
                    capitals.engines.insert("dectalk".into(), cue);
                }
                if let Some(cue) = fallback_cue {
                    capitals.engines.insert("espeak".into(), cue);
                }
                let state = TtsState {
                    current_voice: "paul".into(),
                    pitch_multiplier: 0.8,
                    capital_pitch: Arc::new(capitals),
                    ..TtsState::default()
                };
                let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
                let control = streams.control();
                let generation = AtomicU64::new(1);
                let lifecycle = crate::lifecycle::RequestLifecycle::default();
                let effects = Mutex::new(crate::pipeline::DispatchEffects::new());
                let ctx = crate::pipeline::SynthCtx {
                    letter_navigation: false,
                    gen: 1,
                    gen_counter: &generation,
                    cancellation: None,
                    lifecycle: &lifecycle,
                    engine: primary.as_ref(),
                    control: &control,
                    playback_tickets: None,
                    presentation_clock: None,
                    pending_overlays: None,
                    timeline_renderer: None,
                    effect_processor: Some(&effects),
                    marker_span_id: None,
                    marker_dispatch: None,
                    voice_observations: None,
                    batch_failed: None,
                };
                let outcome = crate::pipeline::process_letter(
                    text,
                    state.clone(),
                    &ctx,
                    &engines,
                    &RuntimeEngineHealth::new(),
                    routes,
                );
                assert!(matches!(outcome, crate::pipeline::BatchStatus::Completed), "primary_streams={primary_streams} fallback_streams={fallback_streams} text={text}: {outcome:?}");
                let first = primary.settings.lock().unwrap();
                let last = fallback.settings.lock().unwrap();
                assert_eq!(first.len(), 1);
                assert_eq!(last.len(), 1);
                assert_eq!(first[0].pitch, first_pitch);
                assert_eq!(last[0].pitch, last_pitch);
                assert_eq!(last[0].rate, state.character_rate());
                assert_eq!(primary.calls.lock().unwrap()[0].0, text.to_lowercase());
                assert_eq!(fallback.calls.lock().unwrap()[0].0, text.to_lowercase());
                assert_eq!(state.pitch_multiplier, 0.8);
            }
        }
    }
}
