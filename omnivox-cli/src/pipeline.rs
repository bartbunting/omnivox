//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, PlaybackTicket,
    PreparedAudioResource, SilenceTrimReport, SilenceTrimmer, StreamType, TimelineAudioRenderer,
    ToneGenerator, VolumeAdjust,
};
use omnivox_core::timeline::{
    ActionAffinity, AudioActionMode, EffectBus, PresentationPosition, ResolvedTimelineAction,
    ScheduledTimeline, TimelineAction, TimelineActionId, TimelineActionKind,
};
use omnivox_core::{QueueItem, TtsState};
use omnivox_tts::contracts::{AcssDimension, PhysicalVoiceId};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::{
    AnchorAffinity, AnchorResolution, RequestedAnchor, ResolvedAnchor, SynthesisMarker,
    SynthesisRequest, SynthesisResult, TtsEngine, TtsSettings,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, warn};

use crate::health::RuntimeEngineHealth;
use crate::marker_events::MarkerDispatchContext;
use crate::routing::{LogicalRoute, LogicalVoiceRoutingSnapshot, RuntimeSynthesisOutcome};
use crate::text::{
    chunk_prepared_speech, extract_logical_voice, extract_pitch, extract_voice,
    prepare_speech_text, rate_scaled_padding, CapitalizationTone,
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
    pub anchors: Vec<ResolvedAnchor>,
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
        anchors: standard.anchors,
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
    pub presentation_clock: Option<&'a Mutex<Vec<PlaybackTicket>>>,
    pub pending_overlays: Option<&'a Mutex<Vec<AudioBuffer>>>,
    pub timeline_renderer: Option<&'a Mutex<TimelineAudioRenderer>>,
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
        if stream == StreamType::Speech {
            self.flush_overlays();
        }
        let needs_ticket = self.playback_tickets.is_some()
            || (stream == StreamType::Speech && self.presentation_clock.is_some());
        let result = if needs_ticket {
            self.control.queue_tracked(stream, buffer).map(|ticket| {
                if let Some(ticket) = ticket {
                    self.record_ticket(stream, ticket);
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

    fn record_ticket(&self, stream: StreamType, ticket: PlaybackTicket) {
        if stream == StreamType::Speech {
            if let Some(clock) = self.presentation_clock {
                clock.lock().unwrap().push(ticket.clone());
            }
        }
        if let Some(tickets) = self.playback_tickets {
            tickets.lock().unwrap().push(ticket);
        }
    }

    pub fn queue_overlay(&self, buffer: AudioBuffer) {
        if let Some(overlays) = self.pending_overlays {
            if !buffer.is_empty() {
                overlays.lock().unwrap().push(buffer);
            }
        } else {
            self.queue(StreamType::Sound, &buffer);
        }
    }

    pub fn flush_overlays(&self) {
        let Some(overlays) = self.pending_overlays else {
            return;
        };
        let buffers = std::mem::take(&mut *overlays.lock().unwrap());
        let Some(buffer) = mix_overlays(buffers) else {
            return;
        };
        let barriers = self
            .presentation_clock
            .map(|clock| clock.lock().unwrap().clone())
            .unwrap_or_default();
        match self.control.queue_overlay_after(&buffer, barriers) {
            Ok(Some(ticket)) => {
                if let Some(tickets) = self.playback_tickets {
                    tickets.lock().unwrap().push(ticket);
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.mark_failed();
                warn!("Overlay queue error: {}", error);
            }
        }
    }
}

fn mix_overlays(buffers: Vec<AudioBuffer>) -> Option<AudioBuffer> {
    let sample_count = buffers
        .iter()
        .map(|buffer| buffer.samples.len())
        .max()
        .unwrap_or(0);
    if sample_count == 0 {
        return None;
    }
    let mut mixed = vec![0.0_f32; sample_count];
    for buffer in buffers {
        for (output, input) in mixed.iter_mut().zip(buffer.samples) {
            *output += input;
        }
    }
    let mut mixed = AudioBuffer::new(mixed);
    mixed.clamp();
    Some(mixed)
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

pub fn synthesize_chunk_with_tones(
    chunk: &str,
    capitalization_tones: &[CapitalizationTone],
    settings: &TtsSettings,
    state: &TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
    ctx: &SynthCtx,
) -> bool {
    if ctx.is_stale() {
        return false;
    }

    let request = SynthesisRequest::new(chunk, settings.clone())
        .with_anchors(requested_capitalization_anchors(capitalization_tones))
        .expect("prepared capitalization offsets are valid");
    match ctx.engine.synthesize(&request).and_then(|mut result| {
        result.resolve_anchors(
            &request,
            ctx.engine.descriptor().capabilities.markers.requested_anchors,
        );
        result.validate(&request)?;
        Ok(result)
    }) {
        Ok(result) => {
            if ctx.is_stale() {
                return false;
            }
            queue_synthesis_result(
                result,
                chunk,
                None,
                capitalization_tones,
                state,
                is_last_speech,
                final_timeline_window,
                ctx,
            );
            true
        }
        Err(e) => {
            ctx.mark_failed();
            warn!("Synthesis error: {}", e);
            true
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_synthesis_result(
    result: SynthesisResult,
    utterance_text: &str,
    logical_voice_id: Option<&str>,
    capitalization_tones: &[CapitalizationTone],
    state: &TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
    ctx: &SynthCtx,
) {
    let mut result = canonicalize_synthesis_result(result);
    debug!(
        engine = %result.engine_id,
        voice = ?result.actual_voice,
        markers = result.markers.len(),
        anchors = result.anchors.len(),
        degraded_acss = ?result.degraded_acss,
        "queueing structured synthesis result"
    );
    if let Err(error) = process_speech_result(&mut result, state, is_last_speech) {
        ctx.mark_failed();
        warn!("Pipeline error: {}", error);
    }
    let overlay_tail = match render_capitalization_timeline(
        &mut result,
        capitalization_tones,
        state,
        final_timeline_window,
        ctx,
    ) {
        Ok(tail) => tail,
        Err(error) => {
            ctx.mark_failed();
            warn!("Timeline render error; queueing dry speech: {}", error);
            None
        }
    };
    if let Some(marker_dispatch) = ctx.marker_dispatch.filter(|_| !result.audio.is_empty()) {
        ctx.flush_overlays();
        let prepared = marker_dispatch.prepare_utterance(
            utterance_text,
            &result.engine_id,
            result.actual_voice.as_ref(),
            logical_voice_id,
            result.audio.sample_rate(),
            result.audio.frame_count(),
            &result.markers,
            &[],
        );
        match prepared.queue(ctx.control, &result.audio) {
            Ok(Some(ticket)) => {
                ctx.record_ticket(StreamType::Speech, ticket);
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
    if let Some(tail) = overlay_tail {
        ctx.queue_overlay(tail);
    }
}

fn requested_capitalization_anchors(tones: &[CapitalizationTone]) -> Vec<RequestedAnchor> {
    tones
        .iter()
        .map(|tone| {
            RequestedAnchor::new(tone.id.clone(), tone.text_offset, AnchorAffinity::Before)
        })
        .collect()
}

fn render_capitalization_timeline(
    result: &mut CanonicalSynthesisResult,
    tones: &[CapitalizationTone],
    state: &TtsState,
    final_window: bool,
    ctx: &SynthCtx,
) -> Result<Option<AudioBuffer>, omnivox_audio::AudioError> {
    let (timeline, resources) = prepare_capitalization_timeline(result, tones, state)?;
    let renderer = ctx.timeline_renderer.ok_or_else(|| {
        omnivox_audio::AudioError::TimelineError("synthesis context has no timeline renderer".into())
    })?;
    let rendered = renderer
        .lock()
        .unwrap()
        .render_window(&result.audio, &timeline, &resources, final_window)?;
    for marker in &mut result.markers {
        marker.frame_offset = rendered.map_primary_frame(marker.frame_offset)?;
    }
    for anchor in &mut result.anchors {
        if let Some(frame_offset) = &mut anchor.frame_offset {
            *frame_offset = rendered.map_primary_frame(*frame_offset)?;
        }
    }
    result.audio = rendered.audio;
    Ok(rendered.overlay_tail)
}

fn prepare_capitalization_timeline(
    result: &CanonicalSynthesisResult,
    tones: &[CapitalizationTone],
    state: &TtsState,
) -> Result<(ScheduledTimeline, Vec<PreparedAudioResource>), omnivox_audio::AudioError> {
    let mut actions = Vec::with_capacity(tones.len());
    let mut resources = Vec::with_capacity(tones.len());
    for tone in tones {
        let resolved = result
            .anchors
            .iter()
            .find(|anchor| anchor.id == tone.id)
            .expect("validated synthesis result contains every requested anchor");
        let frame_offset = resolved.frame_offset.unwrap_or(0);
        if resolved.resolution != AnchorResolution::Exact {
            debug!(
                anchor = %tone.id,
                resolution = ?resolved.resolution,
                frame_offset,
                "capitalization tone placement degraded"
            );
        }
        let id = TimelineActionId::new(tone.id.clone())
            .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
        let mut audio = ToneGenerator::generate(tone.frequency_hz, tone.duration_ms, 1.0);
        build_tone_pipeline(state).process(&mut audio)?;
        let action = TimelineAction {
            id: id.clone(),
            position: PresentationPosition::TextOffset {
                span_id: 0,
                utf8_offset: tone.text_offset,
                affinity: ActionAffinity::Before,
            },
            kind: TimelineActionKind::Audio {
                mode: AudioActionMode::Overlay,
                duration_frames: audio.frame_count() as u64,
                volume: 1.0,
                effect_bus: EffectBus::Dry,
            },
        };
        actions.push(ResolvedTimelineAction {
            action,
            source_frame: frame_offset,
        });
        resources.push(PreparedAudioResource::new(id, audio));
    }
    let timeline = ScheduledTimeline::build(result.audio.frame_count() as u64, actions)
        .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
    Ok((timeline, resources))
}

fn render_primary_window(
    primary: &AudioBuffer,
    final_window: bool,
    ctx: &SynthCtx,
) -> Result<(AudioBuffer, Option<AudioBuffer>), omnivox_audio::AudioError> {
    let timeline = ScheduledTimeline::build(primary.frame_count() as u64, Vec::new())
        .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
    let renderer = ctx.timeline_renderer.ok_or_else(|| {
        omnivox_audio::AudioError::TimelineError("synthesis context has no timeline renderer".into())
    })?;
    let rendered = renderer
        .lock()
        .unwrap()
        .render_window(primary, &timeline, &[], final_window)?;
    Ok((rendered.audio, rendered.overlay_tail))
}

fn finish_timeline_tail(ctx: &SynthCtx) {
    let Some(renderer) = ctx.timeline_renderer else {
        return;
    };
    if !renderer.lock().unwrap().has_overlay_carry() {
        return;
    }
    match render_primary_window(&AudioBuffer::empty(), true, ctx) {
        Ok((_, Some(tail))) => ctx.queue_overlay(tail),
        Ok((_, None)) => {}
        Err(error) => {
            ctx.mark_failed();
            warn!("Could not flush final timeline tail: {}", error);
        }
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
    for anchor in &mut result.anchors {
        if let Some(frame_offset) = &mut anchor.frame_offset {
            *frame_offset = report.map_frame_offset(*frame_offset);
        }
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
    capitalization_tones: &[CapitalizationTone],
    state: &TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
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
    match crate::routing::synthesize_with_runtime_fallback_anchored(
        chunk,
        &requested_capitalization_anchors(capitalization_tones),
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
                capitalization_tones,
                state,
                is_last_speech,
                final_timeline_window,
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
    let chunks = chunk_prepared_speech(prepare_speech_text(text, &state), 15);
    let chunk_count = chunks.len();
    let mut realized = Some(route.realized.clone());
    let mut degraded_acss = route.acss.omitted.clone();

    for (index, chunk) in chunks.into_iter().enumerate() {
        match synthesize_routed_chunk(
            &chunk.text,
            &chunk.capitalization_tones,
            &state,
            index + 1 == chunk_count,
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

    ctx.flush_overlays();

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

    // Pre-count speech chunks for trailing padding and all primary windows for
    // final overlay-tail placement.
    let total_speech_chunks: usize = items
        .iter()
        .map(|item| match item {
            QueueItem::Speech(text) => {
                chunk_prepared_speech(prepare_speech_text(text, &state), 15).len()
            }
            _ => 0,
        })
        .sum();
    let total_primary_windows = total_speech_chunks
        + items
            .iter()
            .filter(|item| matches!(item, QueueItem::Silence { .. }))
            .count();

    let mut speech_chunk_index: usize = 0;
    let mut primary_window_index: usize = 0;
    let mut logical_route: Option<LogicalRoute> = None;
    let mut logical_route_exhausted = false;

    for item in items {
        if ctx.is_stale() {
            return BatchStatus::Cancelled;
        }

        match item {
            QueueItem::Speech(text) => {
                let chunks = chunk_prepared_speech(prepare_speech_text(&text, &state), 15);
                for chunk in chunks {
                    let is_last_speech = speech_chunk_index + 1 == total_speech_chunks;
                    let final_timeline_window = primary_window_index + 1 == total_primary_windows;
                    if logical_route_exhausted {
                        speech_chunk_index += 1;
                        primary_window_index += 1;
                        continue;
                    }
                    if let Some(route) = &mut logical_route {
                        match synthesize_routed_chunk(
                            &chunk.text,
                            &chunk.capitalization_tones,
                            &state,
                            is_last_speech,
                            final_timeline_window,
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
                        if !synthesize_chunk_with_tones(
                            &chunk.text,
                            &chunk.capitalization_tones,
                            &settings,
                            &state,
                            is_last_speech,
                            final_timeline_window,
                            ctx,
                        ) {
                            return BatchStatus::Cancelled;
                        }
                    }
                    speech_chunk_index += 1;
                    primary_window_index += 1;
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
                let final_timeline_window = primary_window_index + 1 == total_primary_windows;
                match render_primary_window(&buf, final_timeline_window, ctx) {
                    Ok((rendered, tail)) => {
                        ctx.queue(StreamType::Speech, &rendered);
                        if let Some(tail) = tail {
                            ctx.queue_overlay(tail);
                        }
                    }
                    Err(error) => {
                        ctx.mark_failed();
                        warn!("Silence timeline render error; queueing dry silence: {}", error);
                        ctx.queue(StreamType::Speech, &buf);
                    }
                }
                primary_window_index += 1;
            }

            QueueItem::AudioIcon { path } => match loader.load(&path) {
                Ok(mut buf) => {
                    let pipeline = build_sound_pipeline(&state);
                    if let Err(e) = pipeline.process(&mut buf) {
                        ctx.mark_failed();
                        warn!("Sound pipeline error: {}", e);
                    }
                    ctx.queue_overlay(buf);
                }
                Err(e) => {
                    ctx.mark_failed();
                    warn!("Failed to load audio icon {}: {}", path.display(), e);
                }
            },
        }
    }

    finish_timeline_tail(ctx);
    ctx.flush_overlays();

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
    use crate::text::{CAPITAL_TONE_DURATION_MS, CAPITAL_TONE_HZ};

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
        result.anchors.push(omnivox_tts::ResolvedAnchor {
            id: "cue".to_owned(),
            frame_offset: Some(100),
            resolution: omnivox_tts::AnchorResolution::Exact,
        });
        result.degraded_acss.push(AcssDimension::PitchRange);

        let canonical = canonicalize_synthesis_result(result);

        assert_eq!(canonical.engine_id, "helper");
        assert_eq!(
            canonical.actual_voice,
            Some(PhysicalVoiceId::new("helper", "voice"))
        );
        assert_eq!(canonical.markers[0].frame_offset, 400);
        assert_eq!(canonical.anchors[0].frame_offset, Some(400));
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
            anchors: vec![omnivox_tts::ResolvedAnchor {
                id: "trimmed-cue".to_owned(),
                frame_offset: Some(6),
                resolution: omnivox_tts::AnchorResolution::Exact,
            }],
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
        assert_eq!(result.anchors[0].frame_offset, Some(1));
    }

    #[test]
    fn test_is_stale() {
        let counter = AtomicU64::new(5);
        assert!(!is_stale(5, &counter));
        assert!(is_stale(4, &counter));
        assert!(is_stale(6, &counter));
    }

    #[test]
    fn same_boundary_overlays_mix_without_advancing_or_serializing() {
        let mixed = mix_overlays(vec![
            AudioBuffer::new(vec![0.75, 0.25, 0.5, -0.5]),
            AudioBuffer::new(vec![0.5, -0.5]),
        ])
        .unwrap();

        assert_eq!(mixed.frame_count(), 2);
        assert_eq!(mixed.samples, vec![1.0, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn capitalization_track_places_tone_at_resolved_frame() {
        let result = CanonicalSynthesisResult {
            audio: AudioBuffer::silence(1.0),
            engine_id: "mock".to_owned(),
            actual_voice: None,
            markers: Vec::new(),
            anchors: vec![ResolvedAnchor {
                id: "capital".to_owned(),
                frame_offset: Some(10),
                resolution: AnchorResolution::Exact,
            }],
            degraded_acss: Vec::new(),
        };
        let tones = vec![CapitalizationTone {
            id: "capital".to_owned(),
            text_offset: 0,
            frequency_hz: CAPITAL_TONE_HZ,
            duration_ms: CAPITAL_TONE_DURATION_MS,
        }];

        let (timeline, resources) =
            prepare_capitalization_timeline(&result, &tones, &TtsState::default()).unwrap();
        let rendered = TimelineAudioRenderer::new()
            .render_window(&result.audio, &timeline, &resources, true)
            .unwrap();

        assert_eq!(timeline.actions[0].output_frame, 10);
        assert_eq!(rendered.audio.frame_count(), 44100);
        assert!(rendered.audio.samples[..20].iter().all(|sample| *sample == 0.0));
        assert!(rendered.audio.samples[20..].iter().any(|sample| *sample != 0.0));
    }

    #[test]
    fn omitted_capitalization_anchor_degrades_to_chunk_start() {
        let result = CanonicalSynthesisResult {
            audio: AudioBuffer::silence(1.0),
            engine_id: "markerless".to_owned(),
            actual_voice: None,
            markers: Vec::new(),
            anchors: vec![ResolvedAnchor {
                id: "capital".to_owned(),
                frame_offset: None,
                resolution: AnchorResolution::Omitted,
            }],
            degraded_acss: Vec::new(),
        };
        let tones = vec![CapitalizationTone {
            id: "capital".to_owned(),
            text_offset: 3,
            frequency_hz: CAPITAL_TONE_HZ,
            duration_ms: CAPITAL_TONE_DURATION_MS,
        }];

        let (timeline, resources) =
            prepare_capitalization_timeline(&result, &tones, &TtsState::default()).unwrap();
        let rendered = TimelineAudioRenderer::new()
            .render_window(&result.audio, &timeline, &resources, true)
            .unwrap();

        assert_eq!(timeline.actions[0].output_frame, 0);
        assert!(rendered.audio.samples.iter().any(|sample| *sample != 0.0));
    }
}
