//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, SilenceTrimmer,
    PlaybackTicket, StreamType, ToneGenerator, VolumeAdjust,
};
use omnivox_core::{QueueItem, TtsState};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::{AudioBuffer as TtsAudioBuffer, TtsEngine, TtsSettings};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, warn};

use crate::health::RuntimeEngineHealth;
use crate::routing::{
    synthesize_with_runtime_fallback, LogicalRoute, LogicalVoiceRoutingSnapshot,
    RuntimeSynthesisOutcome,
};
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
    pub playback_tickets: Option<&'a Mutex<Vec<PlaybackTicket>>>,
    pub batch_failed: Option<&'a AtomicBool>,
}

impl SynthCtx<'_> {
    pub fn is_stale(&self) -> bool {
        is_stale(self.gen, self.gen_counter)
    }

    pub fn mark_failed(&self) {
        if let Some(failed) = self.batch_failed {
            failed.store(true, Ordering::Release);
        }
    }

    pub fn failed(&self) -> bool {
        self.batch_failed
            .is_some_and(|failed| failed.load(Ordering::Acquire))
    }

    pub fn queue(&self, stream: StreamType, buffer: &AudioBuffer) {
        let result = if let Some(tickets) = self.playback_tickets {
            self.control.queue_tracked(stream, buffer).map(|ticket| {
                if let Some(ticket) = ticket {
                    tickets.lock().unwrap().push(ticket);
                }
            })
        } else {
            self.control.queue(stream, buffer).map(|_| ())
        };
        if let Err(error) = result {
            self.mark_failed();
            warn!("{:?} queue error: {}", stream, error);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStatus {
    Completed,
    Cancelled,
    Failed,
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
            queue_synthesized_buffer(tts_buf, state, is_last, ctx);
            true
        }
        Err(e) => {
            ctx.mark_failed();
            warn!("Synthesis error: {}", e);
            true
        }
    }
}

fn queue_synthesized_buffer(
    tts_buf: TtsAudioBuffer,
    state: &TtsState,
    is_last: bool,
    ctx: &SynthCtx,
) {
    let mut buf = tts_buffer_to_audio_buffer(tts_buf);
    let pipeline = build_speech_pipeline(state, is_last);
    if let Err(error) = pipeline.process(&mut buf) {
        ctx.mark_failed();
        warn!("Pipeline error: {}", error);
    }
    ctx.queue(StreamType::Speech, &buf);
}

enum RoutedChunkOutcome {
    Queued,
    Cancelled,
    Failed,
    Exhausted,
}

#[allow(clippy::too_many_arguments)]
fn synthesize_routed_chunk(
    chunk: &str,
    state: &TtsState,
    is_last: bool,
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    ctx: &SynthCtx,
) -> RoutedChunkOutcome {
    let settings = TtsSettings {
        voice: route.realized.voice_id.clone(),
        rate: state.speech_rate,
        pitch: state.pitch_multiplier,
        volume: 1.0,
    };
    match synthesize_with_runtime_fallback(
        chunk,
        &settings,
        route,
        routing,
        engine_registry,
        runtime_health,
        ctx.gen,
        ctx.gen_counter,
    ) {
        RuntimeSynthesisOutcome::Ready(buffer) => {
            queue_synthesized_buffer(buffer, state, is_last, ctx);
            RoutedChunkOutcome::Queued
        }
        RuntimeSynthesisOutcome::Cancelled => RoutedChunkOutcome::Cancelled,
        RuntimeSynthesisOutcome::Failed => RoutedChunkOutcome::Failed,
        RuntimeSynthesisOutcome::Exhausted => RoutedChunkOutcome::Exhausted,
    }
}

/// Process a dispatched batch of queue items in the worker thread.
pub fn process_batch(
    items: Vec<QueueItem>,
    mut state: TtsState,
    ctx: &SynthCtx,
    loader: &AudioFileLoader,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    mut logical_voice_routing: LogicalVoiceRoutingSnapshot,
) -> BatchStatus {
    if ctx.is_stale() {
        return BatchStatus::Cancelled;
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
    let mut logical_route: Option<LogicalRoute> = None;
    let mut logical_route_exhausted = false;

    for item in items {
        if ctx.is_stale() {
            return BatchStatus::Cancelled;
        }

        match item {
            QueueItem::Speech(text) => {
                let processed = preprocess_text(&text, &state);
                let chunks = chunk_text(&processed, 15);
                for chunk in chunks {
                    let is_last = speech_chunk_index == total_speech_chunks - 1;
                    if logical_route_exhausted {
                        speech_chunk_index += 1;
                        continue;
                    }
                    if let Some(route) = &mut logical_route {
                        match synthesize_routed_chunk(
                            &chunk,
                            &state,
                            is_last,
                            route,
                            &mut logical_voice_routing,
                            engine_registry,
                            runtime_health,
                            ctx,
                        ) {
                            RoutedChunkOutcome::Cancelled => return BatchStatus::Cancelled,
                            RoutedChunkOutcome::Failed => ctx.mark_failed(),
                            RoutedChunkOutcome::Exhausted => {
                                ctx.mark_failed();
                                logical_route_exhausted = true;
                            }
                            RoutedChunkOutcome::Queued => {}
                        }
                    } else {
                        let settings = TtsSettings {
                            voice: state.current_voice.clone(),
                            rate: state.speech_rate,
                            pitch: state.pitch_multiplier,
                            volume: 1.0,
                        };
                        if !synthesize_chunk(&chunk, &settings, &state, is_last, ctx) {
                            return BatchStatus::Cancelled;
                        }
                    }
                    speech_chunk_index += 1;
                }
            }

            QueueItem::Code(codes) => {
                if let Some(voice) = extract_voice(&codes) {
                    state.current_voice = voice;
                    logical_route = None;
                    logical_route_exhausted = false;
                }
                if let Some(logical_voice_id) = extract_logical_voice(&codes) {
                    match logical_voice_routing.initial_route(&logical_voice_id, engine_registry) {
                        Ok(route) => {
                            debug!(
                                "Logical voice {} routed to engine {} voice {}",
                                logical_voice_id, route.realized.engine_id, route.realized.voice_id
                            );
                            logical_route = Some(route);
                            logical_route_exhausted = false;
                        }
                        Err(error) => {
                            warn!("{}; using preferred legacy engine", error);
                            logical_route = None;
                            logical_route_exhausted = false;
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
                    ctx.mark_failed();
                    warn!("Tone pipeline error: {}", e);
                }
                ctx.queue(StreamType::Tone, &buf);
            }

            QueueItem::Silence { duration } => {
                let buf = AudioBuffer::silence(duration as f32 / 1000.0);
                ctx.queue(StreamType::Speech, &buf);
            }

            QueueItem::AudioIcon { path } => match loader.load(&path) {
                Ok(mut buf) => {
                    let pipeline = build_sound_pipeline(&state);
                    if let Err(e) = pipeline.process(&mut buf) {
                        ctx.mark_failed();
                        warn!("Sound pipeline error: {}", e);
                    }
                    ctx.queue(StreamType::Sound, &buf);
                }
                Err(e) => {
                    ctx.mark_failed();
                    warn!("Failed to load audio icon {}: {}", path.display(), e);
                }
            },
        }
    }

    if ctx.failed() {
        BatchStatus::Failed
    } else {
        BatchStatus::Completed
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

}
