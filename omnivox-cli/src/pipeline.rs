//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, PlaybackTicket,
    SilenceTrimReport, SilenceTrimmer, StreamType, ToneGenerator, VolumeAdjust,
};
use omnivox_core::{QueueItem, TtsState};
use omnivox_tts::contracts::{AcssDimension, PhysicalVoiceId};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::{SynthesisMarker, SynthesisRequest, SynthesisResult, TtsEngine, TtsSettings};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, warn};

use crate::health::RuntimeEngineHealth;
use crate::marker_events::MarkerDispatchContext;
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

/// A synthesis result whose audio and marker offsets use the pipeline sample rate.
pub struct CanonicalSynthesisResult {
    pub audio: AudioBuffer,
    pub engine_id: String,
    pub actual_voice: Option<PhysicalVoiceId>,
    pub markers: Vec<SynthesisMarker>,
    pub degraded_acss: Vec<AcssDimension>,
}

/// Convert a structured synthesis result to canonical pipeline audio while
/// preserving its realized route and rescaled markers.
pub fn canonicalize_synthesis_result(result: SynthesisResult) -> CanonicalSynthesisResult {
    let standard = result.into_standard_format();
    CanonicalSynthesisResult {
        audio: AudioBuffer::new(standard.audio.samples),
        engine_id: standard.engine_id,
        actual_voice: standard.actual_voice,
        markers: standard.markers,
        degraded_acss: standard.degraded_acss,
    }
}

// ---------------------------------------------------------------------------
// Pipeline builders
// ---------------------------------------------------------------------------

fn speech_trimmer(state: &TtsState, is_last: bool) -> SilenceTrimmer {
    let trailing = if is_last {
        rate_scaled_padding(state.speech_rate)
    } else {
        0.0
    };

    SilenceTrimmer::with_asymmetric_padding(0.01, 0.0, trailing)
}

fn build_speech_output_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.voice_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.speech_routing.channel_mode,
    )));
    pipeline
}

pub fn build_speech_pipeline(state: &TtsState, is_last: bool) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(speech_trimmer(state, is_last)));
    pipeline.push(Box::new(VolumeAdjust::new(state.voice_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.speech_routing.channel_mode,
    )));
    pipeline
}

pub fn build_tone_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.tone_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.tone_routing.channel_mode,
    )));
    pipeline
}

pub fn build_sound_pipeline(state: &TtsState) -> AudioPipeline {
    let mut pipeline = AudioPipeline::new();
    pipeline.push(Box::new(VolumeAdjust::new(state.sound_volume)));
    pipeline.push(Box::new(ChannelRouter::new(
        state.sound_routing.channel_mode,
    )));
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
    pub marker_dispatch: Option<&'a MarkerDispatchContext>,
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

    let request = SynthesisRequest::new(chunk, settings.clone());
    match ctx.engine.synthesize(&request).and_then(|result| {
        result.validate(&request)?;
        Ok(result)
    }) {
        Ok(result) => {
            if ctx.is_stale() {
                return false;
            }
            queue_synthesis_result(result, chunk, None, state, is_last, ctx);
            true
        }
        Err(e) => {
            ctx.mark_failed();
            warn!("Synthesis error: {}", e);
            true
        }
    }
}

fn queue_synthesis_result(
    result: SynthesisResult,
    utterance_text: &str,
    logical_voice_id: Option<&str>,
    state: &TtsState,
    is_last: bool,
    ctx: &SynthCtx,
) {
    let mut result = canonicalize_synthesis_result(result);
    debug!(
        engine = %result.engine_id,
        voice = ?result.actual_voice,
        markers = result.markers.len(),
        degraded_acss = ?result.degraded_acss,
        "queueing structured synthesis result"
    );
    if let Err(error) = process_speech_result(&mut result, state, is_last) {
        ctx.mark_failed();
        warn!("Pipeline error: {}", error);
    }
    if let Some(marker_dispatch) = ctx.marker_dispatch.filter(|_| !result.audio.is_empty()) {
        let prepared = marker_dispatch.prepare_utterance(
            utterance_text,
            &result.engine_id,
            result.actual_voice.as_ref(),
            logical_voice_id,
            result.audio.sample_rate(),
            result.audio.frame_count(),
            &result.markers,
        );
        match prepared.queue(ctx.control, &result.audio) {
            Ok(Some(ticket)) => {
                if let Some(tickets) = ctx.playback_tickets {
                    tickets.lock().unwrap().push(ticket);
                }
            }
            Ok(None) => {}
            Err(error) => {
                ctx.mark_failed();
                warn!("Speech queue error: {}", error);
            }
        }
    } else {
        ctx.queue(StreamType::Speech, &result.audio);
    }
}

fn process_speech_result(
    result: &mut CanonicalSynthesisResult,
    state: &TtsState,
    is_last: bool,
) -> Result<SilenceTrimReport, omnivox_audio::AudioError> {
    let report = speech_trimmer(state, is_last).process_with_report(&mut result.audio)?;
    for marker in &mut result.markers {
        marker.frame_offset = report.map_frame_offset(marker.frame_offset);
    }
    build_speech_output_pipeline(state).process(&mut result.audio)?;
    Ok(report)
}

enum RoutedChunkOutcome {
    Queued {
        realized: PhysicalVoiceId,
        degraded_acss: Vec<AcssDimension>,
    },
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
        RuntimeSynthesisOutcome::Ready(result) => {
            let realized = result
                .actual_voice
                .clone()
                .unwrap_or_else(|| route.realized.clone());
            let degraded_acss = result.degraded_acss.clone();
            queue_synthesis_result(
                *result,
                chunk,
                Some(&route.logical_voice_id),
                state,
                is_last,
                ctx,
            );
            RoutedChunkOutcome::Queued {
                realized,
                degraded_acss,
            }
        }
        RuntimeSynthesisOutcome::Cancelled => RoutedChunkOutcome::Cancelled,
        RuntimeSynthesisOutcome::Failed => RoutedChunkOutcome::Failed,
        RuntimeSynthesisOutcome::Exhausted => RoutedChunkOutcome::Exhausted,
    }
}

/// Terminal synthesis metadata for a non-mutating one-shot preview.
pub struct PreviewSynthesisResult {
    pub status: BatchStatus,
    pub realized: Option<PhysicalVoiceId>,
    pub degraded_acss: Vec<AcssDimension>,
    pub message: Option<String>,
}

/// Resolve, synthesize, and queue one preview without changing persistent TTS
/// or logical-voice state. The caller supplies a private routing snapshot.
pub fn process_preview(
    text: &str,
    state: TtsState,
    ctx: &SynthCtx,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    mut logical_voice_routing: LogicalVoiceRoutingSnapshot,
    logical_voice_id: &str,
) -> PreviewSynthesisResult {
    if ctx.is_stale() {
        return preview_result(BatchStatus::Cancelled, None, Vec::new(), None);
    }

    let mut route = match logical_voice_routing.initial_route(logical_voice_id, engine_registry) {
        Ok(route) => route,
        Err(message) => {
            return preview_result(BatchStatus::Failed, None, Vec::new(), Some(message));
        }
    };
    let processed = preprocess_text(text, &state);
    let chunks = chunk_text(&processed, 15);
    let chunk_count = chunks.len();
    let mut realized = Some(route.realized.clone());
    let mut degraded_acss = route.acss.omitted.clone();

    for (index, chunk) in chunks.into_iter().enumerate() {
        match synthesize_routed_chunk(
            &chunk,
            &state,
            index + 1 == chunk_count,
            &mut route,
            &mut logical_voice_routing,
            engine_registry,
            runtime_health,
            ctx,
        ) {
            RoutedChunkOutcome::Queued {
                realized: chunk_realized,
                degraded_acss: chunk_degraded,
            } => {
                realized = Some(chunk_realized);
                for dimension in chunk_degraded {
                    if !degraded_acss.contains(&dimension) {
                        degraded_acss.push(dimension);
                    }
                }
            }
            RoutedChunkOutcome::Cancelled => {
                return preview_result(BatchStatus::Cancelled, realized, degraded_acss, None);
            }
            RoutedChunkOutcome::Failed => {
                return preview_result(
                    BatchStatus::Failed,
                    realized,
                    degraded_acss,
                    Some("preview synthesis failed".to_owned()),
                );
            }
            RoutedChunkOutcome::Exhausted => {
                return preview_result(
                    BatchStatus::Failed,
                    realized,
                    degraded_acss,
                    Some("preview routing fallback was exhausted".to_owned()),
                );
            }
        }
    }

    let status = if ctx.failed() {
        BatchStatus::Failed
    } else {
        BatchStatus::Completed
    };
    preview_result(status, realized, degraded_acss, None)
}

fn preview_result(
    status: BatchStatus,
    realized: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
    message: Option<String>,
) -> PreviewSynthesisResult {
    PreviewSynthesisResult {
        status,
        realized,
        degraded_acss,
        message,
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
                            RoutedChunkOutcome::Queued { .. } => {}
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

            QueueItem::Tone {
                frequency,
                duration,
            } => {
                let mut buf =
                    ToneGenerator::generate(frequency as f32, duration, state.tone_volume);
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

    fn result(audio: omnivox_tts::AudioBuffer) -> SynthesisResult {
        SynthesisResult::audio("mock", None, audio)
    }

    #[test]
    fn test_canonicalize_synthesis_result() {
        let tts_buf = omnivox_tts::AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2], 44100, 2);
        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;
        assert_eq!(audio_buf.samples, vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(audio_buf.frame_count(), 2);
    }

    #[test]
    fn test_canonicalize_synthesis_result_empty() {
        let tts_buf = omnivox_tts::AudioBuffer::empty();
        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;
        assert!(audio_buf.is_empty());
    }

    #[test]
    fn test_canonicalize_synthesis_result_handles_odd_mono_input() {
        let tts_buf = omnivox_tts::AudioBuffer::new(vec![0.1; 513], 11025, 1);

        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;

        assert!(!audio_buf.is_empty());
        assert_eq!(audio_buf.samples.len() % 2, 0);
        assert_eq!(audio_buf.sample_rate(), omnivox_audio::buffer::SAMPLE_RATE);
        assert_eq!(audio_buf.channels(), omnivox_audio::buffer::CHANNELS);
    }

    #[test]
    fn canonicalization_preserves_metadata_and_rescales_markers() {
        let mut result = SynthesisResult::new(
            "helper",
            Some(PhysicalVoiceId::new("helper", "voice")),
            omnivox_tts::AudioBuffer::new(vec![0.1; 513], 11025, 1),
            vec![SynthesisMarker {
                kind: omnivox_tts::SynthesisMarkerKind::Word,
                frame_offset: 100,
                text_start: Some(0),
                text_length: Some(5),
                value: None,
            }],
        );
        result.degraded_acss.push(AcssDimension::PitchRange);

        let canonical = canonicalize_synthesis_result(result);

        assert_eq!(canonical.engine_id, "helper");
        assert_eq!(
            canonical.actual_voice,
            Some(PhysicalVoiceId::new("helper", "voice"))
        );
        assert_eq!(canonical.markers[0].frame_offset, 400);
        assert_eq!(canonical.degraded_acss, vec![AcssDimension::PitchRange]);
    }

    #[test]
    fn speech_processing_remaps_markers_after_silence_trimming() {
        let mut samples = vec![0.0; 10]; // 5 silent frames
        samples.extend_from_slice(&[0.5, -0.5, 0.3, -0.3]); // 2 audible frames
        samples.extend_from_slice(&[0.0; 6]); // 3 silent frames
        let marker = |frame_offset| SynthesisMarker {
            kind: omnivox_tts::SynthesisMarkerKind::Word,
            frame_offset,
            text_start: None,
            text_length: None,
            value: None,
        };
        let mut result = CanonicalSynthesisResult {
            audio: AudioBuffer::new(samples),
            engine_id: "mock".to_owned(),
            actual_voice: None,
            markers: vec![marker(2), marker(5), marker(6), marker(9)],
            degraded_acss: Vec::new(),
        };

        let report = process_speech_result(&mut result, &TtsState::default(), false).unwrap();

        assert_eq!(report.removed_leading_frames, 5);
        assert_eq!(report.removed_trailing_frames, 3);
        assert_eq!(result.audio.frame_count(), 2);
        assert_eq!(
            result
                .markers
                .iter()
                .map(|marker| marker.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 2]
        );
    }

    #[test]
    fn test_is_stale() {
        let counter = AtomicU64::new(5);
        assert!(!is_stale(5, &counter));
        assert!(is_stale(4, &counter));
        assert!(is_stale(6, &counter));
    }
}
