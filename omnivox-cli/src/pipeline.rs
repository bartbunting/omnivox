//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, SilenceTrimmer,
    StreamType, ToneGenerator, VolumeAdjust,
};
use omnivox_core::{QueueItem, TtsState};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::logical_voices::LogicalVoiceBinding;
use omnivox_tts::{TtsEngine, TtsSettings};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::text::{
    chunk_text, extract_logical_voice, extract_pitch, extract_voice, preprocess_text,
    rate_scaled_padding,
};

// ---------------------------------------------------------------------------
// Buffer conversion
// ---------------------------------------------------------------------------

/// Convert a TTS `AudioBuffer` (omnivox_tts) to the pipeline `AudioBuffer` (omnivox_audio).
pub fn tts_buffer_to_audio_buffer(tts_buf: omnivox_tts::AudioBuffer) -> AudioBuffer {
    if tts_buf.is_empty() {
        return AudioBuffer::empty();
    }
    AudioBuffer::new(tts_buf.samples)
}

// ---------------------------------------------------------------------------
// Pipeline builders
// ---------------------------------------------------------------------------

pub fn build_speech_pipeline(state: &TtsState, is_last: bool) -> AudioPipeline {
    let trailing = if is_last { rate_scaled_padding(state.speech_rate) } else { 0.0 };

    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(SilenceTrimmer::with_asymmetric_padding(
        0.01, 0.0, trailing,
    )));
    pipeline.push(Box::new(VolumeAdjust::new(state.voice_volume)));
    pipeline.push(Box::new(ChannelRouter::new(state.speech_routing.channel_mode)));
    pipeline
}

pub fn build_tone_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.tone_volume)));
    pipeline.push(Box::new(ChannelRouter::new(state.tone_routing.channel_mode)));
    pipeline
}

pub fn build_sound_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.sound_volume)));
    pipeline.push(Box::new(ChannelRouter::new(state.sound_routing.channel_mode)));
    pipeline
}

// ---------------------------------------------------------------------------
// Staleness check and synthesis context
// ---------------------------------------------------------------------------

/// True if the request's generation stamp no longer matches the current counter.
#[inline(always)]
pub fn is_stale(request_gen: u64, gen_counter: &AtomicU64) -> bool {
    gen_counter.load(Ordering::Acquire) != request_gen
}

/// Shared context threaded through all synthesis operations in the worker.
pub struct SynthCtx<'a> {
    pub gen: u64,
    pub gen_counter: &'a AtomicU64,
    pub engine: &'a dyn TtsEngine,
    pub control: &'a AudioControl,
}

impl SynthCtx<'_> {
    pub fn is_stale(&self) -> bool {
        is_stale(self.gen, self.gen_counter)
    }
}

// ---------------------------------------------------------------------------
// Synthesis helpers
// ---------------------------------------------------------------------------

/// Synthesize one text chunk and queue it on the speech stream.
/// Returns `false` if the request was cancelled before or during synthesis.
pub fn synthesize_chunk(
    chunk: &str,
    settings: &TtsSettings,
    state: &TtsState,
    is_last: bool,
    ctx: &SynthCtx,
) -> bool {
    if ctx.is_stale() {
        return false;
    }

    match ctx.engine.synthesize(chunk, settings) {
        Ok(tts_buf) => {
            if ctx.is_stale() {
                return false;
            }
            let mut buf = tts_buffer_to_audio_buffer(tts_buf);
            let pipeline = build_speech_pipeline(state, is_last);
            if let Err(e) = pipeline.process(&mut buf) {
                warn!("Pipeline error: {}", e);
            }
            if let Err(e) = ctx.control.queue(StreamType::Speech, &buf) {
                warn!("Speech queue error: {}", e);
            }
            true
        }
        Err(e) => {
            warn!("Synthesis error: {}", e);
            true
        }
    }
}

fn resolve_logical_route(
    logical_voice_id: &str,
    bindings: &[LogicalVoiceBinding],
    engine_registry: &EngineRegistry,
) -> Result<(Arc<dyn TtsEngine>, PhysicalVoiceId), String> {
    let binding = bindings
        .iter()
        .find(|binding| binding.logical_voice_id() == logical_voice_id)
        .ok_or_else(|| format!("logical voice {logical_voice_id} is not registered"))?;

    match binding {
        LogicalVoiceBinding::Resolved { resolution } => {
            let engine = engine_registry
                .engine(&resolution.realized.engine_id)
                .ok_or_else(|| {
                    format!(
                        "logical voice {logical_voice_id} resolved to missing engine {}",
                        resolution.realized.engine_id
                    )
                })?;
            Ok((engine, resolution.realized.clone()))
        }
        LogicalVoiceBinding::Unresolved { error } => Err(error.to_string()),
    }
}

/// Process a dispatched batch of queue items in the worker thread.
pub fn process_batch(
    items: Vec<QueueItem>,
    mut state: TtsState,
    ctx: &SynthCtx,
    loader: &AudioFileLoader,
    engine_registry: &EngineRegistry,
    logical_voice_bindings: &[LogicalVoiceBinding],
) {
    if ctx.is_stale() {
        return;
    }

    // Pre-count total speech chunks to identify the last one for trailing padding.
    let total_speech_chunks: usize = items
        .iter()
        .map(|item| match item {
            QueueItem::Speech(text) => chunk_text(&preprocess_text(text, &state), 15).len(),
            _ => 0,
        })
        .sum();

    let mut speech_chunk_index: usize = 0;
    let mut logical_route: Option<(Arc<dyn TtsEngine>, PhysicalVoiceId)> = None;

    for item in items {
        if ctx.is_stale() {
            return;
        }

        match item {
            QueueItem::Speech(text) => {
                let engine = logical_route
                    .as_ref()
                    .map_or(ctx.engine, |(engine, _voice_id)| engine.as_ref());
                let settings = TtsSettings {
                    voice: logical_route
                        .as_ref()
                        .map_or_else(
                            || state.current_voice.clone(),
                            |route| route.1.voice_id.clone(),
                        ),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0,
                };
                let routed_ctx = SynthCtx {
                    gen: ctx.gen,
                    gen_counter: ctx.gen_counter,
                    engine,
                    control: ctx.control,
                };
                let processed = preprocess_text(&text, &state);
                let chunks = chunk_text(&processed, 15);
                for chunk in chunks {
                    let is_last = speech_chunk_index == total_speech_chunks - 1;
                    if !synthesize_chunk(&chunk, &settings, &state, is_last, &routed_ctx) {
                        return;
                    }
                    speech_chunk_index += 1;
                }
            }

            QueueItem::Code(codes) => {
                if let Some(voice) = extract_voice(&codes) {
                    state.current_voice = voice;
                    logical_route = None;
                }
                if let Some(logical_voice_id) = extract_logical_voice(&codes) {
                    match resolve_logical_route(
                        &logical_voice_id,
                        logical_voice_bindings,
                        engine_registry,
                    ) {
                        Ok(route) => {
                            debug!(
                                "Logical voice {} routed to engine {} voice {}",
                                logical_voice_id,
                                route.1.engine_id,
                                route.1.voice_id
                            );
                            logical_route = Some(route);
                        }
                        Err(error) => {
                            warn!("{}; using preferred legacy engine", error);
                            logical_route = None;
                        }
                    }
                }
                if let Some(pitch) = extract_pitch(&codes) {
                    state.pitch_multiplier = pitch;
                }
            }

            QueueItem::Tone { frequency, duration } => {
                let mut buf = ToneGenerator::generate(frequency as f32, duration, state.tone_volume);
                let pipeline = build_tone_pipeline(&state);
                if let Err(e) = pipeline.process(&mut buf) {
                    warn!("Tone pipeline error: {}", e);
                }
                if let Err(e) = ctx.control.queue(StreamType::Tone, &buf) {
                    warn!("Tone queue error: {}", e);
                }
            }

            QueueItem::Silence { duration } => {
                let buf = AudioBuffer::silence(duration as f32 / 1000.0);
                if let Err(e) = ctx.control.queue(StreamType::Speech, &buf) {
                    warn!("Silence queue error: {}", e);
                }
            }

            QueueItem::AudioIcon { path } => match loader.load(&path) {
                Ok(mut buf) => {
                    let pipeline = build_sound_pipeline(&state);
                    if let Err(e) = pipeline.process(&mut buf) {
                        warn!("Sound pipeline error: {}", e);
                    }
                    if let Err(e) = ctx.control.queue(StreamType::Sound, &buf) {
                        warn!("Sound queue error: {}", e);
                    }
                }
                Err(e) => warn!("Failed to load audio icon {}: {}", path.display(), e),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::contracts::{
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, EngineDescriptor, EngineHealth, MarkerCapabilities, VoiceDescriptor,
    };
    use omnivox_tts::resolver::{ResolutionReason, VoiceResolution};
    use omnivox_tts::{AudioBuffer as TtsAudioBuffer, TtsError, VoiceInfo, VoiceQuality};

    struct MockEngine {
        descriptor: EngineDescriptor,
    }

    impl TtsEngine for MockEngine {
        fn descriptor(&self) -> EngineDescriptor {
            self.descriptor.clone()
        }

        fn synthesize(
            &self,
            _text: &str,
            _settings: &TtsSettings,
        ) -> Result<TtsAudioBuffer, TtsError> {
            Ok(TtsAudioBuffer::empty())
        }

        fn stop(&self) {}

        fn is_speaking(&self) -> bool {
            false
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
            None
        }
    }

    fn mock_engine(engine_id: &str, voice_id: &str) -> Arc<dyn TtsEngine> {
        Arc::new(MockEngine {
            descriptor: EngineDescriptor {
                id: engine_id.to_owned(),
                display_name: engine_id.to_owned(),
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
                    native_extensions: Vec::new(),
                },
                voices: vec![VoiceDescriptor {
                    id: PhysicalVoiceId::new(engine_id, voice_id),
                    display_name: voice_id.to_owned(),
                    language: Some("en-US".to_owned()),
                    gender: None,
                    quality: VoiceQuality::Compact,
                    availability: Availability::Available,
                }],
                default_voice_id: Some(voice_id.to_owned()),
            },
        })
    }

    #[test]
    fn test_tts_buffer_to_audio_buffer() {
        let tts_buf = omnivox_tts::AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2], 44100, 2);
        let audio_buf = tts_buffer_to_audio_buffer(tts_buf);
        assert_eq!(audio_buf.samples, vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(audio_buf.frame_count(), 2);
    }

    #[test]
    fn test_tts_buffer_to_audio_buffer_empty() {
        let tts_buf = omnivox_tts::AudioBuffer::empty();
        let audio_buf = tts_buffer_to_audio_buffer(tts_buf);
        assert!(audio_buf.is_empty());
    }

    #[test]
    fn test_is_stale() {
        let counter = AtomicU64::new(5);
        assert!(!is_stale(5, &counter));
        assert!(is_stale(4, &counter));
        assert!(is_stale(6, &counter));
    }

    #[test]
    fn logical_route_selects_the_resolved_engine_and_voice() {
        let espeak = mock_engine("espeak", "espeak:en-US");
        let mut registry = EngineRegistry::new();
        registry.register(espeak.clone()).unwrap();
        let realized = PhysicalVoiceId::new("espeak", "espeak:en-US");
        let bindings = vec![LogicalVoiceBinding::Resolved {
            resolution: VoiceResolution {
                logical_voice_id: "source-code".to_owned(),
                requested: None,
                realized: realized.clone(),
                reason: ResolutionReason::Preferred,
                failed_attempts: Vec::new(),
            },
        }];

        let (engine, voice) =
            resolve_logical_route("source-code", &bindings, &registry).unwrap();

        assert!(Arc::ptr_eq(&engine, &espeak));
        assert_eq!(voice, realized);
    }

    #[test]
    fn missing_logical_route_degrades_without_selecting_an_engine() {
        let registry = EngineRegistry::new();

        let error = match resolve_logical_route("missing", &[], &registry) {
            Ok(_) => panic!("missing logical voice unexpectedly resolved"),
            Err(error) => error,
        };

        assert!(error.contains("not registered"));
    }
}
