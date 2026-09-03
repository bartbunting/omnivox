//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    buffer::{CHANNELS, SAMPLE_RATE},
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, PlaybackTicket,
    PostSynthesisParameters, PostSynthesisProcessor, ProgressivePlaybackProducer,
    ProgressiveSilenceTrimmer, SharedPreparedAudioResource, SilenceTrimReport, SilenceTrimmer,
    StreamType, TimelineAudioRenderer, ToneGenerator, VolumeAdjust, MAX_AUDIO_CACHE_SAMPLES,
    MAX_EFFECT_TAIL_FRAMES, MAX_TIMELINE_ACTIONS_PER_WINDOW,
};
use omnivox_core::timeline::{
    ActionAffinity, AudioActionMode, EffectBus, PresentationPosition, ResolvedTimelineAction,
    ScheduledTimeline, TimelineAction, TimelineActionId, TimelineActionKind,
};
use omnivox_core::{ChannelMode, QueueItem, TonePlacement, TtsState};
use omnivox_tts::contracts::{
    apply_rate_offset, AcssDimension, AnchorSupport, AudioOutputMode, NormalizedAcss,
    PhysicalVoiceId, PostSynthesisDimension, PostSynthesisStyle,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::timeline_protocol::{
    PresentationAction, PresentationAffinity, PresentationAudioMode, PresentationEffectBus,
    PresentationEffectDirective, PresentationSpeechSpan, PresentationTimelineAction,
    PresentationTimelineEnvelope, PresentationTimelinePosition,
    MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW,
};
use omnivox_tts::{
    AnchorAffinity, AnchorResolution, RequestedAnchor, ResolvedAnchor, SynthesisCancellationToken,
    SynthesisMarker, SynthesisRequest, SynthesisResult, SynthesisStreamCompletion,
    SynthesisStreamSink, SynthesisStreamStart, TtsEngine, TtsError, TtsSettings,
    MAX_SYNTHESIS_ANCHORS,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::health::RuntimeEngineHealth;
use crate::lifecycle::RequestLifecycle;
use crate::marker_events::{
    MarkerDispatchContext, PlaybackSemanticEvent, PlaybackTimelineResolution,
    PlaybackTimelineResolutionEvent, ProgressiveMarkerPublisher,
};
use crate::routing::{
    legacy_voice_for_engine, LogicalRoute, LogicalVoiceRoutingSnapshot, RuntimeSynthesisOutcome,
};
use crate::text::{
    chunk_prepared_speech, extract_logical_voice, extract_pitch, extract_voice,
    prepare_speech_text, prepare_speech_text_with_offsets, rate_scaled_padding, CapitalizationTone,
    PreparedSpeechChunk,
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

/// Move a canonical structured synthesis result into the pipeline wrapper.
pub fn canonicalize_synthesis_result(result: SynthesisResult) -> CanonicalSynthesisResult {
    CanonicalSynthesisResult {
        audio: result.audio,
        engine_id: result.engine_id,
        actual_voice: result.actual_voice,
        markers: result.markers,
        anchors: result.anchors,
        degraded_acss: result.degraded_acss,
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

fn prepare_tone_audio(
    frequency_hz: f32,
    duration_ms: u32,
    state: &TtsState,
) -> Result<AudioBuffer, omnivox_audio::AudioError> {
    let mut audio = ToneGenerator::generate(frequency_hz, duration_ms, 1.0);
    build_tone_pipeline(state).process(&mut audio)?;
    Ok(audio)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToneQueueTarget {
    Stream(StreamType),
    Overlay,
}

fn tone_queue_target(placement: TonePlacement) -> ToneQueueTarget {
    match placement {
        TonePlacement::Independent => ToneQueueTarget::Stream(StreamType::Tone),
        TonePlacement::Insert => ToneQueueTarget::Stream(StreamType::Speech),
        TonePlacement::Overlay => ToneQueueTarget::Overlay,
    }
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
    pub cancellation: Option<&'a SynthesisCancellationToken>,
    pub lifecycle: &'a RequestLifecycle,
    pub engine: &'a dyn TtsEngine,
    pub control: &'a AudioControl,
    pub playback_tickets: Option<&'a Mutex<Vec<PlaybackTicket>>>,
    pub presentation_clock: Option<&'a Mutex<Vec<PlaybackTicket>>>,
    pub pending_overlays: Option<&'a Mutex<Vec<AudioBuffer>>>,
    pub timeline_renderer: Option<&'a Mutex<TimelineAudioRenderer>>,
    pub effect_processor: Option<&'a Mutex<PostSynthesisProcessor>>,
    pub marker_dispatch: Option<&'a MarkerDispatchContext>,
    pub batch_failed: Option<&'a AtomicBool>,
}

impl SynthCtx<'_> {
    pub fn is_stale(&self) -> bool {
        is_stale(self.gen, self.gen_counter)
            || self
                .cancellation
                .is_some_and(SynthesisCancellationToken::is_cancelled)
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
        let queue_attempted_at = Instant::now();
        let needs_ticket = self.playback_tickets.is_some()
            || (stream == StreamType::Speech && self.presentation_clock.is_some());
        let result = if let Some(cancellation) = self.cancellation {
            self.control
                .queue_tracked_cancellable_if(stream, buffer, cancellation.clone(), || {
                    !self.is_stale()
                })
                .map(|ticket| {
                    let queued = ticket.is_some();
                    if let Some(ticket) = ticket {
                        self.record_ticket(stream, ticket);
                    }
                    queued
                })
        } else if needs_ticket {
            self.control
                .queue_tracked_if(stream, buffer, || !self.is_stale())
                .map(|ticket| {
                    let queued = ticket.is_some();
                    if let Some(ticket) = ticket {
                        self.record_ticket(stream, ticket);
                    }
                    queued
                })
        } else {
            self.control.queue_if(stream, buffer, || !self.is_stale())
        };
        match result {
            Ok(true) => self.lifecycle.record_audio_queued_at(queue_attempted_at),
            Ok(false) => {}
            Err(error) => {
                self.mark_failed();
                warn!("{:?} queue error: {}", stream, error);
            }
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
        if self.is_stale() {
            return;
        }
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
        let queued = if let Some(cancellation) = self.cancellation {
            self.control.queue_overlay_after_cancellable_if(
                &buffer,
                barriers,
                cancellation.clone(),
                || !self.is_stale(),
            )
        } else {
            self.control
                .queue_overlay_after_if(&buffer, barriers, || !self.is_stale())
        };
        match queued {
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

fn attach_synthesis_cancellation(
    mut request: SynthesisRequest,
    ctx: &SynthCtx,
) -> SynthesisRequest {
    request.cancellation = ctx.cancellation.cloned();
    request
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

#[derive(Debug, Clone)]
struct TimelineChunkAction {
    id: String,
    text_offset: u32,
    affinity: AnchorAffinity,
    kind: TimelineChunkActionKind,
}

#[derive(Debug, Clone)]
enum TimelineChunkActionKind {
    Audio {
        resource: TimelineAudioResource,
        mode: AudioActionMode,
        volume: f32,
        effect_bus: EffectBus,
    },
    SemanticEvent,
}

#[derive(Debug, Clone)]
enum TimelineAudioResource {
    File {
        audio: Arc<AudioBuffer>,
        pan: f32,
    },
    Tone {
        frequency_hz: f32,
        duration_ms: u32,
        pan: f32,
    },
    Silence {
        duration_ms: u32,
    },
}

const MAX_PRESENTATION_DECODED_PCM_SAMPLES: usize = MAX_AUDIO_CACHE_SAMPLES;

#[derive(Debug, PartialEq, Eq)]
enum TimelinePreparationError {
    Cancelled,
    Invalid(String),
}

impl fmt::Display for TimelinePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("structured presentation preparation cancelled"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

struct PresentationPcmBudget {
    limit: usize,
    retained: usize,
    shared_allocations: HashSet<usize>,
}

impl PresentationPcmBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            retained: 0,
            shared_allocations: HashSet::new(),
        }
    }

    fn retain_shared(
        &mut self,
        action_id: &str,
        audio: &Arc<AudioBuffer>,
    ) -> Result<(), TimelinePreparationError> {
        let allocation = Arc::as_ptr(audio) as usize;
        if self.shared_allocations.contains(&allocation) {
            return Ok(());
        }
        self.reserve(action_id, audio.samples.len())?;
        self.shared_allocations.insert(allocation);
        Ok(())
    }

    fn retain_private(
        &mut self,
        action_id: &str,
        samples: usize,
    ) -> Result<(), TimelinePreparationError> {
        self.reserve(action_id, samples)
    }

    fn reserve(&mut self, action_id: &str, samples: usize) -> Result<(), TimelinePreparationError> {
        let attempted = self.retained.saturating_add(samples);
        if attempted > self.limit {
            return Err(TimelinePreparationError::Invalid(format!(
                "action {action_id} would retain {attempted} decoded PCM samples; presentation maximum is {}",
                self.limit
            )));
        }
        self.retained = attempted;
        Ok(())
    }
}

fn check_timeline_preparation_cancelled(
    cancelled: &dyn Fn() -> bool,
) -> Result<(), TimelinePreparationError> {
    if cancelled() {
        Err(TimelinePreparationError::Cancelled)
    } else {
        Ok(())
    }
}

impl TimelineAudioResource {
    fn materialize(&self, state: &TtsState) -> Result<Arc<AudioBuffer>, omnivox_audio::AudioError> {
        match self {
            Self::File { audio, pan } => {
                if state.sound_volume == 1.0
                    && state.sound_routing.channel_mode == ChannelMode::Both
                    && *pan == 0.5
                {
                    return Ok(Arc::clone(audio));
                }
                let mut prepared = (**audio).clone();
                build_sound_pipeline(state).process(&mut prepared)?;
                apply_action_pan(&mut prepared, *pan);
                Ok(Arc::new(prepared))
            }
            Self::Tone {
                frequency_hz,
                duration_ms,
                pan,
            } => {
                let mut audio = prepare_tone_audio(*frequency_hz, *duration_ms, state)?;
                apply_action_pan(&mut audio, *pan);
                Ok(Arc::new(audio))
            }
            Self::Silence { duration_ms } => {
                Ok(Arc::new(AudioBuffer::silence(*duration_ms as f32 / 1000.0)))
            }
        }
    }
}

fn canonical_samples_for_duration_ms(duration_ms: u32) -> usize {
    (SAMPLE_RATE as usize)
        .saturating_mul(duration_ms as usize)
        .saturating_div(1000)
        .saturating_mul(CHANNELS as usize)
}

fn effect_tail_samples(effect_bus: PresentationEffectBus) -> usize {
    if effect_bus == PresentationEffectBus::Speech {
        MAX_EFFECT_TAIL_FRAMES.saturating_mul(CHANNELS as usize)
    } else {
        0
    }
}

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

    let request = attach_synthesis_cancellation(
        SynthesisRequest::new(chunk, settings.clone())
            .with_anchors(requested_timeline_anchors(capitalization_tones, &[]))
            .expect("prepared capitalization offsets are valid"),
        ctx,
    );
    let engine_id = ctx.engine.descriptor().id;
    let synthesis_started_at = Instant::now();
    info!(
        lifecycle_stage = "synthesis_started",
        engine_id = %engine_id,
        text_bytes = chunk.len(),
        "Speech lifecycle started direct synthesis"
    );
    if descriptor_supports_progressive_anchors(
        &ctx.engine.descriptor(),
        &request.anchors,
        ctx.timeline_renderer.is_some(),
    ) {
        let synthesis = synthesize_direct_chunk_progressively(
            &request,
            chunk,
            None,
            capitalization_tones,
            &[],
            PostSynthesisStyle::default(),
            Vec::new(),
            state,
            is_last_speech,
            final_timeline_window,
            ctx,
        );
        return match synthesis {
            Ok(_) if ctx.is_stale() => false,
            Ok(_) => true,
            Err(error) => {
                warn!(
                    lifecycle_stage = "synthesis_failed",
                    engine_id = %engine_id,
                    synthesis_elapsed_us = u64::try_from(
                        synthesis_started_at.elapsed().as_micros()
                    )
                    .unwrap_or(u64::MAX),
                    error = %error,
                    "Speech lifecycle failed direct progressive synthesis"
                );
                ctx.mark_failed();
                true
            }
        };
    }
    let synthesis = ctx.engine.synthesize(&request).and_then(|mut result| {
        result.resolve_anchors(
            &request,
            ctx.engine
                .descriptor()
                .capabilities
                .markers
                .requested_anchors,
        );
        result.validate(&request)?;
        Ok(result)
    });
    match synthesis {
        Ok(result) => {
            info!(
                lifecycle_stage = "synthesis_completed",
                engine_id = %engine_id,
                frames = result.audio.frame_count(),
                synthesis_elapsed_us = u64::try_from(
                    synthesis_started_at.elapsed().as_micros()
                )
                .unwrap_or(u64::MAX),
                "Speech lifecycle completed direct synthesis"
            );
            if ctx.is_stale() {
                return false;
            }
            queue_synthesis_result(
                result,
                chunk,
                None,
                capitalization_tones,
                &[],
                &PostSynthesisStyle::default(),
                &[],
                state,
                is_last_speech,
                final_timeline_window,
                ctx,
            );
            !ctx.is_stale()
        }
        Err(e) => {
            warn!(
                lifecycle_stage = "synthesis_failed",
                engine_id = %engine_id,
                synthesis_elapsed_us = u64::try_from(
                    synthesis_started_at.elapsed().as_micros()
                )
                .unwrap_or(u64::MAX),
                error = %e,
                "Speech lifecycle failed direct synthesis"
            );
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
    timeline_actions: &[TimelineChunkAction],
    effects: &PostSynthesisStyle,
    degraded_effects: &[PostSynthesisDimension],
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
    let effect_tail =
        match process_effect_window(&mut result.audio, effects, final_timeline_window, ctx) {
            Ok(tail) => tail,
            Err(error) => {
                ctx.mark_failed();
                warn!(
                    "Post-synthesis effect error; queueing dry speech: {}",
                    error
                );
                None
            }
        };
    let (overlay_tail, semantic_events) = match render_speech_timeline(
        &mut result,
        capitalization_tones,
        timeline_actions,
        effects,
        state,
        final_timeline_window,
        ctx,
    ) {
        Ok(rendered) => rendered,
        Err(error) => {
            ctx.mark_failed();
            warn!("Timeline render error; queueing dry speech: {}", error);
            (None, Vec::new())
        }
    };
    if let Some(marker_dispatch) = ctx.marker_dispatch.filter(|_| !result.audio.is_empty()) {
        ctx.flush_overlays();
        let prepared = if marker_dispatch.supports_timeline_events() {
            let resolutions = timeline_actions
                .iter()
                .filter_map(|action| {
                    result
                        .anchors
                        .iter()
                        .find(|anchor| anchor.id == action.id)
                        .and_then(|anchor| {
                            TimelineActionId::new(action.id.clone())
                                .ok()
                                .map(|action_id| PlaybackTimelineResolution {
                                    action_id,
                                    resolution: anchor.resolution,
                                })
                        })
                })
                .collect::<Vec<_>>();
            marker_dispatch.prepare_timeline_utterance(
                utterance_text,
                &result.engine_id,
                result.actual_voice.as_ref(),
                logical_voice_id,
                result.audio.sample_rate(),
                result.audio.frame_count(),
                &result.markers,
                &semantic_events,
                &resolutions,
                &result.degraded_acss,
                degraded_effects,
            )
        } else {
            marker_dispatch.prepare_utterance(
                utterance_text,
                &result.engine_id,
                result.actual_voice.as_ref(),
                logical_voice_id,
                result.audio.sample_rate(),
                result.audio.frame_count(),
                &result.markers,
                &semantic_events,
            )
        };
        let queued = if let Some(cancellation) = ctx.cancellation {
            prepared.queue_cancellable_if(ctx.control, &result.audio, cancellation.clone(), || {
                !ctx.is_stale()
            })
        } else {
            prepared.queue_if(ctx.control, &result.audio, || !ctx.is_stale())
        };
        match queued {
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
    if let Some(tail) = effect_tail {
        ctx.queue_overlay(tail);
    }
}

fn post_synthesis_parameters(style: &PostSynthesisStyle) -> PostSynthesisParameters {
    let style = style.clone().clamped();
    let logarithmic =
        |minimum: f32, maximum: f32, value: f32| minimum * (maximum / minimum).powf(value);
    PostSynthesisParameters {
        gain: style
            .gain
            .map(|value| 10.0_f32.powf(((value - 0.5) * 24.0) / 20.0))
            .unwrap_or(1.0),
        low_pass_hz: style
            .low_pass
            .map(|value| logarithmic(200.0, 20_000.0, value)),
        high_pass_hz: style
            .high_pass
            .map(|value| logarithmic(20.0, 3_000.0, value)),
        pan: style.pan.map(|value| value * 2.0 - 1.0).unwrap_or(0.0),
        chorus: style.chorus.unwrap_or(0.0),
        reverb: style.reverb.unwrap_or(0.0),
        echo: style.echo.unwrap_or(0.0),
    }
}

fn process_effect_window(
    audio: &mut AudioBuffer,
    effects: &PostSynthesisStyle,
    final_window: bool,
    ctx: &SynthCtx,
) -> Result<Option<AudioBuffer>, omnivox_audio::AudioError> {
    let processor = ctx.effect_processor.ok_or_else(|| {
        omnivox_audio::AudioError::EffectError(
            "synthesis context has no post-synthesis processor".into(),
        )
    })?;
    let processed = processor.lock().unwrap().process_window(
        audio,
        post_synthesis_parameters(effects),
        final_window,
    );
    *audio = processed.audio;
    Ok(processed.tail)
}

fn requested_timeline_anchors(
    tones: &[CapitalizationTone],
    actions: &[TimelineChunkAction],
) -> Vec<RequestedAnchor> {
    tones
        .iter()
        .map(|tone| RequestedAnchor::new(tone.id.clone(), tone.text_offset, AnchorAffinity::Before))
        .chain(actions.iter().map(|action| {
            RequestedAnchor::new(action.id.clone(), action.text_offset, action.affinity)
        }))
        .collect()
}

fn with_timeline_anchors(
    request: SynthesisRequest,
    tones: &[CapitalizationTone],
    actions: &[TimelineChunkAction],
) -> Result<SynthesisRequest, omnivox_tts::TtsError> {
    request.with_anchors(requested_timeline_anchors(tones, actions))
}

fn render_speech_timeline(
    result: &mut CanonicalSynthesisResult,
    tones: &[CapitalizationTone],
    actions: &[TimelineChunkAction],
    effects: &PostSynthesisStyle,
    state: &TtsState,
    final_window: bool,
    ctx: &SynthCtx,
) -> Result<(Option<AudioBuffer>, Vec<PlaybackSemanticEvent>), omnivox_audio::AudioError> {
    let (timeline, resources) = prepare_speech_timeline(result, tones, actions, effects, state)?;
    let renderer = ctx.timeline_renderer.ok_or_else(|| {
        omnivox_audio::AudioError::TimelineError(
            "synthesis context has no timeline renderer".into(),
        )
    })?;
    let rendered = renderer.lock().unwrap().render_shared_window(
        &result.audio,
        &timeline,
        &resources,
        final_window,
    )?;
    for marker in &mut result.markers {
        marker.frame_offset = rendered.map_primary_frame(marker.frame_offset)?;
    }
    for anchor in &mut result.anchors {
        if let Some(frame_offset) = &mut anchor.frame_offset {
            *frame_offset = rendered.map_primary_frame(*frame_offset)?;
        }
    }
    result.audio = rendered.audio;
    Ok((
        rendered.overlay_tail,
        rendered
            .semantic_events
            .into_iter()
            .map(|event| PlaybackSemanticEvent {
                action_id: event.id,
                frame_offset: event.frame_offset,
            })
            .collect(),
    ))
}

fn prepare_speech_timeline(
    result: &CanonicalSynthesisResult,
    tones: &[CapitalizationTone],
    timeline_actions: &[TimelineChunkAction],
    effects: &PostSynthesisStyle,
    state: &TtsState,
) -> Result<(ScheduledTimeline, Vec<SharedPreparedAudioResource>), omnivox_audio::AudioError> {
    let (actions, resources) =
        prepare_speech_timeline_actions(tones, timeline_actions, effects, state)?;
    let actions = actions
        .into_iter()
        .map(|action| {
            let resolved = result
                .anchors
                .iter()
                .find(|anchor| anchor.id == action.id.as_str())
                .expect("validated synthesis result contains every requested anchor");
            let frame_offset = resolved.frame_offset.unwrap_or_else(|| {
                if matches!(
                    action.position,
                    PresentationPosition::TextOffset {
                        affinity: ActionAffinity::After,
                        ..
                    } | PresentationPosition::SpanBoundary {
                        affinity: ActionAffinity::After,
                        ..
                    }
                ) {
                    result.audio.frame_count() as u64
                } else {
                    0
                }
            });
            if resolved.resolution != AnchorResolution::Exact {
                debug!(
                    anchor = %action.id,
                    resolution = ?resolved.resolution,
                    frame_offset,
                    "timeline placement degraded"
                );
            }
            ResolvedTimelineAction {
                action,
                source_frame: frame_offset,
            }
        })
        .collect::<Vec<_>>();
    let timeline = ScheduledTimeline::build(result.audio.frame_count() as u64, actions)
        .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
    Ok((timeline, resources))
}

fn prepare_speech_timeline_actions(
    tones: &[CapitalizationTone],
    timeline_actions: &[TimelineChunkAction],
    effects: &PostSynthesisStyle,
    state: &TtsState,
) -> Result<(Vec<TimelineAction>, Vec<SharedPreparedAudioResource>), omnivox_audio::AudioError> {
    let mut actions = Vec::with_capacity(tones.len() + timeline_actions.len());
    let mut resources = Vec::with_capacity(tones.len() + timeline_actions.len());
    for tone in tones {
        let id = TimelineActionId::new(tone.id.clone())
            .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
        let audio = prepare_tone_audio(tone.frequency_hz, tone.duration_ms, state)?;
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
        actions.push(action);
        resources.push(SharedPreparedAudioResource::new(id, Arc::new(audio)));
    }
    for action in timeline_actions {
        let id = TimelineActionId::new(action.id.clone())
            .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
        let kind = match &action.kind {
            TimelineChunkActionKind::Audio {
                resource,
                mode,
                volume,
                effect_bus,
            } => {
                let mut audio = resource.materialize(state)?;
                if audio.is_empty() {
                    return Err(omnivox_audio::AudioError::TimelineError(format!(
                        "action {} materialized to empty audio",
                        action.id
                    )));
                }
                if *effect_bus == EffectBus::Speech {
                    let mut processor = PostSynthesisProcessor::new();
                    let processed =
                        processor.process_window(&audio, post_synthesis_parameters(effects), true);
                    let mut processed_audio = processed.audio;
                    if let Some(tail) = processed.tail {
                        processed_audio.append(&tail);
                    }
                    audio = Arc::new(processed_audio);
                }
                let duration_frames = audio.frame_count() as u64;
                resources.push(SharedPreparedAudioResource::new(id.clone(), audio));
                TimelineActionKind::Audio {
                    mode: *mode,
                    duration_frames,
                    volume: *volume,
                    effect_bus: *effect_bus,
                }
            }
            TimelineChunkActionKind::SemanticEvent => TimelineActionKind::SemanticEvent,
        };
        actions.push(TimelineAction {
            id,
            position: PresentationPosition::TextOffset {
                span_id: 0,
                utf8_offset: action.text_offset,
                affinity: match action.affinity {
                    AnchorAffinity::Before => ActionAffinity::Before,
                    AnchorAffinity::After => ActionAffinity::After,
                },
            },
            kind,
        });
    }
    Ok((actions, resources))
}

fn render_primary_window(
    primary: &AudioBuffer,
    effects: &PostSynthesisStyle,
    final_window: bool,
    ctx: &SynthCtx,
) -> Result<(AudioBuffer, Vec<AudioBuffer>), omnivox_audio::AudioError> {
    let mut primary = primary.clone();
    let effect_tail = process_effect_window(&mut primary, effects, final_window, ctx)?;
    let timeline = ScheduledTimeline::build(primary.frame_count() as u64, Vec::new())
        .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
    let renderer = ctx.timeline_renderer.ok_or_else(|| {
        omnivox_audio::AudioError::TimelineError(
            "synthesis context has no timeline renderer".into(),
        )
    })?;
    let rendered =
        renderer
            .lock()
            .unwrap()
            .render_window(&primary, &timeline, &[], final_window)?;
    Ok((
        rendered.audio,
        [effect_tail, rendered.overlay_tail]
            .into_iter()
            .flatten()
            .collect(),
    ))
}

fn finish_timeline_tail(ctx: &SynthCtx) {
    let Some(renderer) = ctx.timeline_renderer else {
        return;
    };
    if !renderer.lock().unwrap().has_overlay_carry() {
        return;
    }
    match render_primary_window(
        &AudioBuffer::empty(),
        &PostSynthesisStyle::default(),
        true,
        ctx,
    ) {
        Ok((_, tails)) => tails.into_iter().for_each(|tail| ctx.queue_overlay(tail)),
        Err(error) => {
            ctx.mark_failed();
            warn!("Could not flush final timeline tail: {}", error);
        }
    }
}

fn finish_effect_tail(ctx: &SynthCtx) {
    let Some(processor) = ctx.effect_processor else {
        return;
    };
    if let Some(tail) = processor.lock().unwrap().finish() {
        ctx.queue_overlay(tail);
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

struct CompletedProgressiveChunk {
    actual_voice: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
}

struct ProgressiveRenderedWindow {
    audio: AudioBuffer,
    overlay_tail: Option<AudioBuffer>,
    resolution_events: Vec<PlaybackTimelineResolutionEvent>,
    semantic_events: Vec<PlaybackSemanticEvent>,
}

struct ProgressiveTimelineAction {
    order: usize,
    source_frame: u64,
    resolution: AnchorResolution,
}

struct ProgressiveChunkSink<'a, 'ctx> {
    utterance_text: &'a str,
    logical_voice_id: Option<String>,
    effects: PostSynthesisStyle,
    degraded_effects: Vec<PostSynthesisDimension>,
    state: &'a TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
    ctx: &'a SynthCtx<'ctx>,
    start: Option<SynthesisStreamStart>,
    trimmer: Option<ProgressiveSilenceTrimmer>,
    producer: Option<ProgressivePlaybackProducer>,
    marker_publisher: Option<ProgressiveMarkerPublisher>,
    pending_markers: VecDeque<SynthesisMarker>,
    marker_count: usize,
    last_marker_offset: Option<u64>,
    timeline_actions: Vec<TimelineAction>,
    timeline_action_indices: HashMap<String, usize>,
    reported_timeline_action_ids: HashSet<String>,
    resolved_timeline_actions: Vec<ProgressiveTimelineAction>,
    resolved_timeline_action_ids: HashSet<String>,
    last_timeline_anchor_offset: Option<u64>,
    timeline_insertions: Vec<(u64, u64)>,
    timeline_resources: Vec<SharedPreparedAudioResource>,
    primary_frame_count: u64,
    output_frame_count: u64,
    ticket: Option<PlaybackTicket>,
}

impl<'a, 'ctx> ProgressiveChunkSink<'a, 'ctx> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        utterance_text: &'a str,
        logical_voice_id: Option<&str>,
        effects: PostSynthesisStyle,
        degraded_effects: Vec<PostSynthesisDimension>,
        state: &'a TtsState,
        is_last_speech: bool,
        final_timeline_window: bool,
        capitalization_tones: &[CapitalizationTone],
        timeline_actions: &[TimelineChunkAction],
        ctx: &'a SynthCtx<'ctx>,
    ) -> Result<Self, TtsError> {
        let reported_timeline_action_ids = timeline_actions
            .iter()
            .map(|action| action.id.clone())
            .collect();
        let (timeline_actions, timeline_resources) = prepare_speech_timeline_actions(
            capitalization_tones,
            timeline_actions,
            &effects,
            state,
        )
        .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        let timeline_action_indices = timeline_actions
            .iter()
            .enumerate()
            .map(|(index, action)| (action.id.as_str().to_owned(), index))
            .collect();
        Ok(Self {
            utterance_text,
            logical_voice_id: logical_voice_id.map(str::to_owned),
            effects,
            degraded_effects,
            state,
            is_last_speech,
            final_timeline_window,
            ctx,
            start: None,
            trimmer: None,
            producer: None,
            marker_publisher: None,
            pending_markers: VecDeque::new(),
            marker_count: 0,
            last_marker_offset: None,
            timeline_actions,
            timeline_action_indices,
            reported_timeline_action_ids,
            resolved_timeline_actions: Vec::new(),
            resolved_timeline_action_ids: HashSet::new(),
            last_timeline_anchor_offset: None,
            timeline_insertions: Vec::new(),
            timeline_resources,
            primary_frame_count: 0,
            output_frame_count: 0,
            ticket: None,
        })
    }

    fn ensure_playback(&mut self) -> Result<&mut ProgressivePlaybackProducer, TtsError> {
        if self.producer.is_none() {
            let start = self.start.as_ref().ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive engine emitted audio before stream metadata".to_owned(),
                )
            })?;
            self.ctx.flush_overlays();
            let cancellation = self
                .ctx
                .cancellation
                .cloned()
                .unwrap_or_else(SynthesisCancellationToken::new);
            if let Some(marker_dispatch) = self.ctx.marker_dispatch {
                let prepared = if marker_dispatch.supports_timeline_events() {
                    marker_dispatch.prepare_timeline_utterance(
                        self.utterance_text,
                        &start.engine_id,
                        start.actual_voice.as_ref(),
                        self.logical_voice_id.as_deref(),
                        SAMPLE_RATE,
                        0,
                        &[],
                        &[],
                        &[],
                        &start.degraded_acss,
                        &self.degraded_effects,
                    )
                } else {
                    marker_dispatch.prepare_utterance(
                        self.utterance_text,
                        &start.engine_id,
                        start.actual_voice.as_ref(),
                        self.logical_voice_id.as_deref(),
                        SAMPLE_RATE,
                        0,
                        &[],
                        &[],
                    )
                };
                let queued = prepared
                    .queue_progressive_cancellable_if(self.ctx.control, cancellation, || {
                        !self.ctx.is_stale()
                    })
                    .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
                let Some((producer, ticket, marker_publisher)) = queued else {
                    return Err(TtsError::SynthesisFailed(
                        "progressive playback was superseded before queueing".to_owned(),
                    ));
                };
                self.producer = Some(producer);
                self.marker_publisher = Some(marker_publisher);
                self.ticket = Some(ticket);
            } else {
                let queued_at = Instant::now();
                let queued = self
                    .ctx
                    .control
                    .queue_progressive_speech_with_cue_callback_cancellable_if(
                        |_| {},
                        cancellation,
                        || !self.ctx.is_stale(),
                    )
                    .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
                if queued.is_some() {
                    self.ctx.lifecycle.record_audio_queued_at(queued_at);
                }
                let Some((producer, ticket)) = queued else {
                    return Err(TtsError::SynthesisFailed(
                        "progressive playback was superseded before queueing".to_owned(),
                    ));
                };
                self.producer = Some(producer);
                self.ticket = Some(ticket);
            }
        }
        Ok(self.producer.as_mut().unwrap())
    }

    fn register_timeline_anchors(&mut self, anchors: Vec<ResolvedAnchor>) -> Result<(), TtsError> {
        for anchor in anchors {
            let Some(&order) = self.timeline_action_indices.get(&anchor.id) else {
                return Err(TtsError::SynthesisFailed(format!(
                    "progressive engine resolved unknown timeline anchor {}",
                    anchor.id
                )));
            };
            if anchor.resolution == AnchorResolution::Omitted {
                return Err(TtsError::SynthesisFailed(format!(
                    "progressive engine omitted timeline anchor {}",
                    anchor.id
                )));
            }
            let source_frame = anchor.frame_offset.ok_or_else(|| {
                TtsError::SynthesisFailed(format!(
                    "progressive engine returned no frame for {:?} timeline anchor {}",
                    anchor.resolution, anchor.id
                ))
            })?;
            if self
                .last_timeline_anchor_offset
                .is_some_and(|previous| source_frame < previous)
            {
                return Err(TtsError::SynthesisFailed(
                    "progressive timeline anchors are out of order".to_owned(),
                ));
            }
            if !self.resolved_timeline_action_ids.insert(anchor.id.clone()) {
                return Err(TtsError::SynthesisFailed(format!(
                    "progressive engine resolved timeline anchor {} more than once",
                    anchor.id
                )));
            }
            self.last_timeline_anchor_offset = Some(source_frame);
            if let TimelineActionKind::Audio {
                mode: AudioActionMode::Insert,
                duration_frames,
                ..
            } = &self.timeline_actions[order].kind
            {
                self.timeline_insertions
                    .push((source_frame, *duration_frames));
            }
            self.resolved_timeline_actions
                .push(ProgressiveTimelineAction {
                    order,
                    source_frame,
                    resolution: anchor.resolution,
                });
        }
        Ok(())
    }

    fn map_primary_frame(
        &self,
        source_frame: u64,
        final_report: Option<&SilenceTrimReport>,
    ) -> Option<u64> {
        if let Some(report) = final_report {
            return Some(report.map_frame_offset(source_frame));
        }
        self.trimmer
            .as_ref()
            .and_then(ProgressiveSilenceTrimmer::removed_leading_frames)
            .map(|removed| source_frame.saturating_sub(removed as u64))
    }

    fn map_output_frame(
        &self,
        source_frame: u64,
        final_report: Option<&SilenceTrimReport>,
    ) -> Result<Option<u64>, TtsError> {
        let Some(primary_frame) = self.map_primary_frame(source_frame, final_report) else {
            return Ok(None);
        };
        self.timeline_insertions
            .iter()
            .try_fold(
                primary_frame,
                |output_frame, (insertion_frame, duration)| {
                    let insertion_frame = self
                        .map_primary_frame(*insertion_frame, final_report)
                        .expect("primary frame mapping availability is shared");
                    if insertion_frame <= primary_frame {
                        output_frame.checked_add(*duration).ok_or_else(|| {
                            TtsError::SynthesisFailed(
                                "progressive timeline frame mapping overflowed".to_owned(),
                            )
                        })
                    } else {
                        Ok(output_frame)
                    }
                },
            )
            .map(Some)
    }

    fn take_ready_timeline_actions(
        &mut self,
        source_end: u64,
        include_end_actions: bool,
        final_report: Option<&SilenceTrimReport>,
    ) -> Result<Vec<(ResolvedTimelineAction, AnchorResolution)>, TtsError> {
        let source_start = self.primary_frame_count;
        let mut pending = Vec::new();
        let mut ready = Vec::new();
        let resolved = std::mem::take(&mut self.resolved_timeline_actions);
        for resolved in resolved {
            let source_frame = self
                .map_primary_frame(resolved.source_frame, final_report)
                .expect("trim mapping is known before timeline audio is rendered");
            if source_frame < source_start {
                return Err(TtsError::SynthesisFailed(format!(
                    "progressive timeline action {} at frame {source_frame} arrived after frame {source_start}",
                    self.timeline_actions[resolved.order].id
                )));
            }
            if source_frame < source_end || (include_end_actions && source_frame == source_end) {
                ready.push((resolved.order, source_frame, resolved.resolution));
            } else {
                pending.push(resolved);
            }
        }
        self.resolved_timeline_actions = pending;
        ready.sort_by_key(|(order, source_frame, _)| (*source_frame, *order));
        Ok(ready
            .into_iter()
            .map(|(order, source_frame, resolution)| {
                (
                    ResolvedTimelineAction {
                        action: self.timeline_actions[order].clone(),
                        source_frame,
                    },
                    resolution,
                )
            })
            .collect())
    }

    fn render_timeline_window(
        &mut self,
        audio: AudioBuffer,
        include_end_actions: bool,
        final_timeline_window: bool,
        final_report: Option<&SilenceTrimReport>,
    ) -> Result<ProgressiveRenderedWindow, TtsError> {
        let source_start = self.primary_frame_count;
        let source_end = source_start
            .checked_add(audio.frame_count() as u64)
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive primary timeline frame count overflowed".to_owned(),
                )
            })?;
        let resolved =
            self.take_ready_timeline_actions(source_end, include_end_actions, final_report)?;
        let actions = resolved
            .iter()
            .map(|(action, _)| action.clone())
            .collect::<Vec<_>>();
        let (audio, overlay_tail, resolution_events, semantic_events) =
            if let Some(renderer) = self.ctx.timeline_renderer {
                let rendered = renderer
                    .lock()
                    .unwrap()
                    .render_incremental_shared_window(
                        &audio,
                        source_start,
                        &actions,
                        &self.timeline_resources,
                        final_timeline_window,
                    )
                    .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
                let resolution_events = resolved
                    .into_iter()
                    .filter_map(|(action, resolution)| {
                        self.reported_timeline_action_ids
                            .contains(action.action.id.as_str())
                            .then(|| {
                                Ok(PlaybackTimelineResolutionEvent {
                                    action_id: action.action.id,
                                    resolution,
                                    frame_offset: rendered
                                        .map_primary_frame(action.source_frame - source_start)?,
                                })
                            })
                    })
                    .collect::<Result<Vec<_>, omnivox_audio::AudioError>>()
                    .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
                (
                    rendered.audio,
                    rendered.overlay_tail,
                    resolution_events,
                    rendered.semantic_events,
                )
            } else if actions.is_empty() {
                (audio, None, Vec::new(), Vec::new())
            } else {
                return Err(TtsError::SynthesisFailed(
                    "synthesis context has no progressive timeline renderer".to_owned(),
                ));
            };
        let output_start = self.output_frame_count;
        self.primary_frame_count = source_end;
        self.output_frame_count = self
            .output_frame_count
            .checked_add(audio.frame_count() as u64)
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive output timeline frame count overflowed".to_owned(),
                )
            })?;
        let semantic_events = semantic_events
            .into_iter()
            .map(|event| {
                Ok(PlaybackSemanticEvent {
                    action_id: event.id,
                    frame_offset: output_start.checked_add(event.frame_offset).ok_or_else(
                        || {
                            TtsError::SynthesisFailed(
                                "progressive semantic event frame overflowed".to_owned(),
                            )
                        },
                    )?,
                })
            })
            .collect::<Result<Vec<_>, TtsError>>()?;
        let resolution_events = resolution_events
            .into_iter()
            .map(|mut event| {
                event.frame_offset =
                    output_start
                        .checked_add(event.frame_offset)
                        .ok_or_else(|| {
                            TtsError::SynthesisFailed(
                                "progressive timeline resolution frame overflowed".to_owned(),
                            )
                        })?;
                Ok(event)
            })
            .collect::<Result<Vec<_>, TtsError>>()?;
        Ok(ProgressiveRenderedWindow {
            audio,
            overlay_tail,
            resolution_events,
            semantic_events,
        })
    }

    fn publish_events_through(
        &mut self,
        output_frame: u64,
        final_report: Option<&SilenceTrimReport>,
        mut resolution_events: Vec<PlaybackTimelineResolutionEvent>,
        mut semantic_events: Vec<PlaybackSemanticEvent>,
    ) -> Result<(), TtsError> {
        let Some(marker_dispatch) = self.ctx.marker_dispatch else {
            self.pending_markers.clear();
            return Ok(());
        };
        if !marker_dispatch.supports_timeline_events() {
            resolution_events.clear();
            semantic_events.clear();
        }
        if self.map_primary_frame(0, final_report).is_none() {
            return Ok(());
        }
        let mut ready = Vec::new();
        while let Some(marker) = self.pending_markers.front() {
            let mapped = self
                .map_output_frame(marker.frame_offset, final_report)?
                .expect("trim mapping availability was checked");
            if mapped > output_frame {
                break;
            }
            let mut marker = self.pending_markers.pop_front().unwrap();
            marker.frame_offset = mapped;
            ready.push(marker);
        }
        if ready.is_empty() && resolution_events.is_empty() && semantic_events.is_empty() {
            return Ok(());
        }
        let producer = self.producer.as_mut().ok_or_else(|| {
            TtsError::SynthesisFailed(
                "progressive markers became playable before audio was queued".to_owned(),
            )
        })?;
        let publisher = self.marker_publisher.as_mut().ok_or_else(|| {
            TtsError::SynthesisFailed(
                "progressive marker dispatch was not prepared before audio".to_owned(),
            )
        })?;
        if !publisher
            .push_timeline_events(
                marker_dispatch,
                producer,
                ready,
                resolution_events,
                semantic_events,
                || !self.ctx.is_stale(),
            )
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?
        {
            return Err(TtsError::SynthesisFailed(
                "progressive marker playback was superseded".to_owned(),
            ));
        }
        Ok(())
    }

    fn process_audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        let mut audio = self
            .trimmer
            .as_mut()
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive engine emitted audio before stream metadata".to_owned(),
                )
            })?
            .process_window(audio)
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        if audio.is_empty() {
            return Ok(());
        }
        build_speech_output_pipeline(self.state)
            .process(&mut audio)
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        let unexpected_tail = process_effect_window(&mut audio, &self.effects, false, self.ctx)
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        debug_assert!(unexpected_tail.is_none());
        let rendered = self.render_timeline_window(audio, false, false, None)?;
        debug_assert!(rendered.overlay_tail.is_none());
        self.ensure_playback()?;
        self.publish_events_through(
            self.output_frame_count,
            None,
            rendered.resolution_events,
            rendered.semantic_events,
        )?;
        self.producer
            .as_mut()
            .unwrap()
            .push_audio(rendered.audio)
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))
    }

    fn finish(
        &mut self,
        completion: SynthesisStreamCompletion,
    ) -> Result<CompletedProgressiveChunk, TtsError> {
        let (mut tail, report) = self
            .trimmer
            .as_mut()
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive engine completed before stream metadata".to_owned(),
                )
            })?
            .finish()
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        if report.input_frames as u64 != completion.frame_count {
            return Err(TtsError::SynthesisFailed(format!(
                "progressive pipeline received {} frames but engine reported {}",
                report.input_frames, completion.frame_count
            )));
        }
        if self
            .last_marker_offset
            .is_some_and(|offset| offset > completion.frame_count)
        {
            return Err(TtsError::SynthesisFailed(
                "progressive engine marker exceeds the completed PCM".to_owned(),
            ));
        }
        if self
            .last_timeline_anchor_offset
            .is_some_and(|offset| offset > completion.frame_count)
        {
            return Err(TtsError::SynthesisFailed(
                "progressive timeline anchor exceeds the completed PCM".to_owned(),
            ));
        }
        if self.resolved_timeline_action_ids.len() != self.timeline_actions.len() {
            return Err(TtsError::SynthesisFailed(format!(
                "progressive route resolved {} of {} timeline anchors",
                self.resolved_timeline_action_ids.len(),
                self.timeline_actions.len()
            )));
        }
        build_speech_output_pipeline(self.state)
            .process(&mut tail)
            .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        let effect_tail = process_effect_window(
            &mut tail,
            &self.effects,
            self.final_timeline_window,
            self.ctx,
        )
        .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        let rendered =
            self.render_timeline_window(tail, true, self.final_timeline_window, Some(&report))?;
        if self.primary_frame_count != report.output_frames as u64 {
            return Err(TtsError::SynthesisFailed(format!(
                "progressive timeline rendered {} primary frames but trimming produced {}",
                self.primary_frame_count, report.output_frames
            )));
        }
        if !self.resolved_timeline_actions.is_empty() {
            return Err(TtsError::SynthesisFailed(
                "progressive timeline left actions beyond completed playback".to_owned(),
            ));
        }
        if self.output_frame_count > 0 {
            self.ensure_playback()?;
            self.publish_events_through(
                self.output_frame_count,
                Some(&report),
                rendered.resolution_events,
                rendered.semantic_events,
            )?;
        } else {
            self.pending_markers.clear();
        }
        if !self.pending_markers.is_empty() {
            return Err(TtsError::SynthesisFailed(
                "progressive engine left markers beyond completed playback".to_owned(),
            ));
        }
        if !rendered.audio.is_empty() {
            self.producer
                .as_mut()
                .unwrap()
                .push_audio(rendered.audio)
                .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
        }
        if let Some(producer) = self.producer.take() {
            let ticket = self.ticket.take().expect("queued producer has a ticket");
            producer
                .finish()
                .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
            self.ctx.record_ticket(StreamType::Speech, ticket);
        }
        if let Some(tail) = rendered.overlay_tail {
            self.ctx.queue_overlay(tail);
        }
        if let Some(tail) = effect_tail {
            self.ctx.queue_overlay(tail);
        }
        let start = self.start.take().ok_or_else(|| {
            TtsError::SynthesisFailed("progressive engine omitted stream metadata".to_owned())
        })?;
        Ok(CompletedProgressiveChunk {
            actual_voice: start.actual_voice,
            degraded_acss: start.degraded_acss,
        })
    }
}

impl SynthesisStreamSink for ProgressiveChunkSink<'_, '_> {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        if self.start.is_some() {
            return Err(TtsError::SynthesisFailed(
                "progressive engine emitted stream metadata more than once".to_owned(),
            ));
        }
        self.start = Some(start);
        self.trimmer = Some(ProgressiveSilenceTrimmer::with_asymmetric_padding(
            0.01,
            0.0,
            if self.is_last_speech {
                rate_scaled_padding(self.state.speech_rate)
            } else {
                0.0
            },
        ));
        Ok(())
    }

    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.process_audio(audio)
    }

    fn markers(
        &mut self,
        markers: Vec<SynthesisMarker>,
        anchors: Vec<ResolvedAnchor>,
    ) -> Result<(), TtsError> {
        if markers.is_empty() && anchors.is_empty() {
            return Ok(());
        }
        if self.start.is_none() {
            return Err(TtsError::SynthesisFailed(
                "progressive engine emitted markers before stream metadata".to_owned(),
            ));
        }
        self.register_timeline_anchors(anchors)?;
        if self.ctx.marker_dispatch.is_none() {
            return Ok(());
        }
        self.marker_count = self
            .marker_count
            .checked_add(markers.len())
            .filter(|count| *count <= 4096)
            .ok_or_else(|| {
                TtsError::SynthesisFailed(
                    "progressive engine emitted more than 4096 markers".to_owned(),
                )
            })?;
        for marker in &markers {
            if self
                .last_marker_offset
                .is_some_and(|offset| marker.frame_offset < offset)
            {
                return Err(TtsError::SynthesisFailed(
                    "progressive engine markers are out of order".to_owned(),
                ));
            }
            self.last_marker_offset = Some(marker.frame_offset);
        }
        self.pending_markers.extend(markers);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn synthesize_direct_chunk_progressively(
    request: &SynthesisRequest,
    utterance_text: &str,
    logical_voice_id: Option<&str>,
    capitalization_tones: &[CapitalizationTone],
    timeline_actions: &[TimelineChunkAction],
    effects: PostSynthesisStyle,
    degraded_effects: Vec<PostSynthesisDimension>,
    state: &TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
    ctx: &SynthCtx,
) -> Result<CompletedProgressiveChunk, TtsError> {
    let mut sink = ProgressiveChunkSink::new(
        utterance_text,
        logical_voice_id,
        effects,
        degraded_effects,
        state,
        is_last_speech,
        final_timeline_window,
        capitalization_tones,
        timeline_actions,
        ctx,
    )?;
    let completion = ctx.engine.synthesize_stream(request, &mut sink)?;
    sink.finish(completion)
}

fn route_supports_initial_progressive_playback(
    route: &LogicalRoute,
    anchors: &[RequestedAnchor],
    ctx: &SynthCtx,
) -> bool {
    descriptor_supports_progressive_anchors(
        &route.engine.descriptor(),
        anchors,
        ctx.timeline_renderer.is_some(),
    )
}

fn descriptor_supports_progressive_anchors(
    descriptor: &omnivox_tts::contracts::EngineDescriptor,
    anchors: &[RequestedAnchor],
    timeline_renderer_available: bool,
) -> bool {
    descriptor.capabilities.audio_output == AudioOutputMode::StreamingPcm
        && (anchors.is_empty()
            || (timeline_renderer_available
                && descriptor.capabilities.markers.requested_anchors != AnchorSupport::None))
}

enum RoutedChunkOutcome {
    Queued {
        realized: PhysicalVoiceId,
        degraded_acss: Vec<AcssDimension>,
        degraded_effects: Vec<PostSynthesisDimension>,
    },
    Cancelled,
    Failed,
    Exhausted,
}

#[allow(clippy::too_many_arguments)]
fn synthesize_routed_chunk(
    chunk: &str,
    capitalization_tones: &[CapitalizationTone],
    timeline_actions: &[TimelineChunkAction],
    requested_acss: Option<&NormalizedAcss>,
    requested_effects: Option<&PostSynthesisStyle>,
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
    let anchors = requested_timeline_anchors(capitalization_tones, timeline_actions);
    if route_supports_initial_progressive_playback(route, &anchors, ctx) {
        return synthesize_routed_chunk_progressively(
            chunk,
            &anchors,
            &settings,
            requested_acss,
            requested_effects,
            state,
            is_last_speech,
            final_timeline_window,
            capitalization_tones,
            timeline_actions,
            route,
            routing,
            engine_registry,
            runtime_health,
            ctx,
        );
    }
    let outcome = if let Some(requested_acss) = requested_acss {
        crate::routing::synthesize_with_runtime_fallback_anchored_styled(
            chunk,
            &anchors,
            &settings,
            requested_acss,
            route,
            routing,
            engine_registry,
            runtime_health,
            ctx.gen,
            ctx.gen_counter,
            ctx.cancellation,
        )
    } else {
        crate::routing::synthesize_with_runtime_fallback_anchored(
            chunk,
            &anchors,
            &settings,
            route,
            routing,
            engine_registry,
            runtime_health,
            ctx.gen,
            ctx.gen_counter,
            ctx.cancellation,
        )
    };
    match outcome {
        RuntimeSynthesisOutcome::Ready(result) => {
            let realized = result
                .actual_voice
                .clone()
                .unwrap_or_else(|| route.realized.clone());
            let degraded_acss = result.degraded_acss.clone();
            let effect_application = requested_effects
                .cloned()
                .unwrap_or_else(|| route.effects.style.clone())
                .degrade_for(
                    &route
                        .engine
                        .descriptor()
                        .capabilities
                        .post_synthesis_dimensions,
                );
            let degraded_effects = effect_application.omitted.clone();
            queue_synthesis_result(
                *result,
                chunk,
                route.reported_logical_voice_id.as_deref(),
                capitalization_tones,
                timeline_actions,
                &effect_application.style,
                &degraded_effects,
                state,
                is_last_speech,
                final_timeline_window,
                ctx,
            );
            if ctx.is_stale() {
                RoutedChunkOutcome::Cancelled
            } else {
                RoutedChunkOutcome::Queued {
                    realized,
                    degraded_acss,
                    degraded_effects,
                }
            }
        }
        RuntimeSynthesisOutcome::Cancelled => RoutedChunkOutcome::Cancelled,
        RuntimeSynthesisOutcome::Failed => RoutedChunkOutcome::Failed,
        RuntimeSynthesisOutcome::Exhausted => RoutedChunkOutcome::Exhausted,
    }
}

#[allow(clippy::too_many_arguments)]
fn synthesize_routed_chunk_progressively(
    chunk: &str,
    anchors: &[RequestedAnchor],
    settings: &TtsSettings,
    requested_acss: Option<&NormalizedAcss>,
    requested_effects: Option<&PostSynthesisStyle>,
    state: &TtsState,
    is_last_speech: bool,
    final_timeline_window: bool,
    capitalization_tones: &[CapitalizationTone],
    timeline_actions: &[TimelineChunkAction],
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    ctx: &SynthCtx,
) -> RoutedChunkOutcome {
    let initial_effect_application = requested_effects
        .cloned()
        .unwrap_or_else(|| route.effects.style.clone())
        .degrade_for(
            &route
                .engine
                .descriptor()
                .capabilities
                .post_synthesis_dimensions,
        );
    let mut sink = match ProgressiveChunkSink::new(
        chunk,
        route.reported_logical_voice_id.as_deref(),
        initial_effect_application.style,
        initial_effect_application.omitted,
        state,
        is_last_speech,
        final_timeline_window,
        capitalization_tones,
        timeline_actions,
        ctx,
    ) {
        Ok(sink) => sink,
        Err(error) => {
            ctx.mark_failed();
            warn!("Progressive timeline preparation failed: {error}");
            return RoutedChunkOutcome::Failed;
        }
    };
    let outcome = crate::routing::synthesize_progressively_with_runtime_fallback_anchored(
        chunk,
        anchors,
        settings,
        requested_acss,
        route,
        routing,
        engine_registry,
        runtime_health,
        ctx.gen,
        ctx.gen_counter,
        ctx.cancellation,
        &mut sink,
    );
    match outcome {
        crate::routing::RuntimeProgressiveSynthesisOutcome::Streamed(completion) => {
            match sink.finish(completion) {
                Ok(_completed) if ctx.is_stale() => RoutedChunkOutcome::Cancelled,
                Ok(completed) => RoutedChunkOutcome::Queued {
                    realized: completed
                        .actual_voice
                        .unwrap_or_else(|| route.realized.clone()),
                    degraded_acss: completed.degraded_acss,
                    degraded_effects: sink.degraded_effects.clone(),
                },
                Err(error) => {
                    ctx.mark_failed();
                    warn!("Progressive speech pipeline failed: {error}");
                    RoutedChunkOutcome::Failed
                }
            }
        }
        crate::routing::RuntimeProgressiveSynthesisOutcome::Buffered(result) => {
            let realized = result
                .actual_voice
                .clone()
                .unwrap_or_else(|| route.realized.clone());
            let degraded_acss = result.degraded_acss.clone();
            let effect_application = requested_effects
                .cloned()
                .unwrap_or_else(|| route.effects.style.clone())
                .degrade_for(
                    &route
                        .engine
                        .descriptor()
                        .capabilities
                        .post_synthesis_dimensions,
                );
            let degraded_effects = effect_application.omitted.clone();
            queue_synthesis_result(
                *result,
                chunk,
                route.reported_logical_voice_id.as_deref(),
                capitalization_tones,
                timeline_actions,
                &effect_application.style,
                &degraded_effects,
                state,
                is_last_speech,
                final_timeline_window,
                ctx,
            );
            if ctx.is_stale() {
                RoutedChunkOutcome::Cancelled
            } else {
                RoutedChunkOutcome::Queued {
                    realized,
                    degraded_acss,
                    degraded_effects,
                }
            }
        }
        crate::routing::RuntimeProgressiveSynthesisOutcome::Cancelled => {
            RoutedChunkOutcome::Cancelled
        }
        crate::routing::RuntimeProgressiveSynthesisOutcome::Failed => RoutedChunkOutcome::Failed,
        crate::routing::RuntimeProgressiveSynthesisOutcome::Exhausted => {
            RoutedChunkOutcome::Exhausted
        }
    }
}

fn initial_legacy_route(
    state: &TtsState,
    ctx: &SynthCtx,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
) -> Option<LogicalRoute> {
    let engine_id = ctx.engine.descriptor().id;
    match routing.initial_legacy_route(
        PhysicalVoiceId::new(engine_id, state.current_voice.clone()),
        engine_registry,
    ) {
        Ok(route) => Some(route),
        Err(error) => {
            warn!("Could not establish implicit legacy route: {error}");
            None
        }
    }
}

const ISOLATED_CAPITAL_PITCH_MULTIPLIER: f32 = 1.5;

fn prepare_isolated_letter(text: &str, state: &mut TtsState) -> String {
    if text.chars().next().is_some_and(char::is_uppercase) {
        // Preserve the established character-review cue independently of the
        // Aural presentation selected for capitalization in words and lines.
        state.pitch_multiplier = ISOLATED_CAPITAL_PITCH_MULTIPLIER;
    }
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Speak one character through the same runtime fallback path as queued speech.
pub fn process_letter(
    text: &str,
    mut state: TtsState,
    ctx: &SynthCtx,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    mut routing: LogicalVoiceRoutingSnapshot,
) -> BatchStatus {
    if ctx.is_stale() {
        return BatchStatus::Cancelled;
    }
    state.current_voice = legacy_voice_for_engine(ctx.engine, &state.current_voice);
    state.speech_rate = state.character_rate();
    let letter = prepare_isolated_letter(text, &mut state);
    let status = if let Some(mut content_route) =
        initial_legacy_route(&state, ctx, &mut routing, engine_registry)
    {
        match synthesize_routed_chunk(
            &letter,
            &[],
            &[],
            None,
            None,
            &state,
            true,
            true,
            &mut content_route,
            &mut routing,
            engine_registry,
            runtime_health,
            ctx,
        ) {
            RoutedChunkOutcome::Queued { .. } => BatchStatus::Completed,
            RoutedChunkOutcome::Cancelled => BatchStatus::Cancelled,
            RoutedChunkOutcome::Failed | RoutedChunkOutcome::Exhausted => BatchStatus::Failed,
        }
    } else {
        let settings = TtsSettings {
            voice: state.current_voice.clone(),
            rate: state.speech_rate,
            pitch: state.pitch_multiplier,
            volume: 1.0,
        };
        if synthesize_chunk_with_tones(&letter, &[], &settings, &state, true, true, ctx) {
            BatchStatus::Completed
        } else {
            BatchStatus::Cancelled
        }
    };
    ctx.flush_overlays();
    status
}

/// Terminal synthesis metadata for a non-mutating one-shot preview.
pub struct PreviewSynthesisResult {
    pub status: BatchStatus,
    pub realized: Option<PhysicalVoiceId>,
    pub degraded_acss: Vec<AcssDimension>,
    pub degraded_effects: Vec<PostSynthesisDimension>,
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
        return preview_result(BatchStatus::Cancelled, None, Vec::new(), Vec::new(), None);
    }

    let mut route = match logical_voice_routing.initial_route(logical_voice_id, engine_registry) {
        Ok(route) => route,
        Err(message) => {
            return preview_result(
                BatchStatus::Failed,
                None,
                Vec::new(),
                Vec::new(),
                Some(message),
            );
        }
    };
    let chunks = chunk_prepared_speech(prepare_speech_text(text, &state), 15);
    let chunk_count = chunks.len();
    let mut realized = Some(route.realized.clone());
    let mut degraded_acss = route.acss.omitted.clone();
    let mut degraded_effects = route.effects.omitted.clone();

    for (index, chunk) in chunks.into_iter().enumerate() {
        match synthesize_routed_chunk(
            &chunk.text,
            &chunk.capitalization_tones,
            &[],
            None,
            None,
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
                degraded_effects: chunk_degraded_effects,
            } => {
                realized = Some(chunk_realized);
                for dimension in chunk_degraded {
                    if !degraded_acss.contains(&dimension) {
                        degraded_acss.push(dimension);
                    }
                }
                for dimension in chunk_degraded_effects {
                    if !degraded_effects.contains(&dimension) {
                        degraded_effects.push(dimension);
                    }
                }
            }
            RoutedChunkOutcome::Cancelled => {
                return preview_result(
                    BatchStatus::Cancelled,
                    realized,
                    degraded_acss,
                    degraded_effects,
                    None,
                );
            }
            RoutedChunkOutcome::Failed => {
                return preview_result(
                    BatchStatus::Failed,
                    realized,
                    degraded_acss,
                    degraded_effects,
                    Some("preview synthesis failed".to_owned()),
                );
            }
            RoutedChunkOutcome::Exhausted => {
                return preview_result(
                    BatchStatus::Failed,
                    realized,
                    degraded_acss,
                    degraded_effects,
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
    preview_result(status, realized, degraded_acss, degraded_effects, None)
}

fn preview_result(
    status: BatchStatus,
    realized: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
    degraded_effects: Vec<PostSynthesisDimension>,
    message: Option<String>,
) -> PreviewSynthesisResult {
    PreviewSynthesisResult {
        status,
        realized,
        degraded_acss,
        degraded_effects,
        message,
    }
}

#[derive(Debug)]
struct PreparedTimelineSpan {
    id: u64,
    logical_voice_id: Option<String>,
    acss: NormalizedAcss,
    effects: PresentationEffectDirective,
    chunks: Vec<PreparedSpeechChunk>,
    actions: Vec<Vec<TimelineChunkAction>>,
}

#[derive(Debug, Clone, Copy)]
struct PreparedTimelineActionPosition {
    chunk_index: usize,
    local_offset: u32,
    affinity: AnchorAffinity,
}

#[derive(Debug)]
struct PreparedTimelineSpanLayout {
    chunks: Vec<PreparedSpeechChunk>,
    action_positions: Vec<PreparedTimelineActionPosition>,
}

/// Validate and share file resources up front, then synthesize and queue one
/// structured, tracked presentation timeline. Bounded generated resources are
/// materialized only for the render window that owns them.
pub fn process_presentation_timeline(
    timeline: PresentationTimelineEnvelope,
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
    state.current_voice = legacy_voice_for_engine(ctx.engine, &state.current_voice);
    let cancelled = || ctx.is_stale();
    let resources = match prepare_timeline_resources(&timeline.actions, &state, loader, &cancelled)
    {
        Ok(resources) => resources,
        Err(TimelinePreparationError::Cancelled) => return BatchStatus::Cancelled,
        Err(TimelinePreparationError::Invalid(error)) => {
            ctx.mark_failed();
            warn!("Structured presentation resource validation failed: {error}");
            return BatchStatus::Failed;
        }
    };
    let spans = match prepare_timeline_spans(&timeline, &state, &resources, &cancelled) {
        Ok(spans) => spans,
        Err(TimelinePreparationError::Cancelled) => return BatchStatus::Cancelled,
        Err(TimelinePreparationError::Invalid(error)) => {
            ctx.mark_failed();
            warn!("Structured presentation preparation failed: {error}");
            return BatchStatus::Failed;
        }
    };
    let total_chunks = spans.iter().map(|span| span.chunks.len()).sum::<usize>();
    let mut chunk_index = 0_usize;
    let mut active_effects: Option<PostSynthesisStyle> = None;

    for span in spans {
        match span.effects {
            PresentationEffectDirective::Retain => {}
            PresentationEffectDirective::Replace { style, .. } => active_effects = Some(style),
            PresentationEffectDirective::End => {
                active_effects = Some(PostSynthesisStyle::default())
            }
        }
        let mut route = match span.logical_voice_id.as_deref() {
            Some(logical_voice_id) => {
                match logical_voice_routing.initial_route(logical_voice_id, engine_registry) {
                    Ok(route) => Some(route),
                    Err(error) => {
                        warn!(
                            "{error}; using the preferred legacy engine for span {}",
                            span.id
                        );
                        None
                    }
                }
            }
            None => None,
        };
        if route.is_none() {
            route = initial_legacy_route(&state, ctx, &mut logical_voice_routing, engine_registry);
        }
        let requested_acss = acss_has_values(&span.acss).then_some(&span.acss);
        for (chunk, actions) in span.chunks.into_iter().zip(span.actions) {
            if ctx.is_stale() {
                return BatchStatus::Cancelled;
            }
            let final_window = chunk_index + 1 == total_chunks;
            let queued = if let Some(route) = &mut route {
                match synthesize_routed_chunk(
                    &chunk.text,
                    &chunk.capitalization_tones,
                    &actions,
                    requested_acss,
                    active_effects.as_ref(),
                    &state,
                    final_window,
                    final_window,
                    route,
                    &mut logical_voice_routing,
                    engine_registry,
                    runtime_health,
                    ctx,
                ) {
                    RoutedChunkOutcome::Queued { .. } => true,
                    RoutedChunkOutcome::Cancelled => return BatchStatus::Cancelled,
                    RoutedChunkOutcome::Failed | RoutedChunkOutcome::Exhausted => {
                        ctx.mark_failed();
                        true
                    }
                }
            } else {
                synthesize_direct_timeline_chunk(
                    &chunk,
                    &actions,
                    requested_acss,
                    active_effects.as_ref(),
                    &state,
                    final_window,
                    ctx,
                )
            };
            if !queued {
                return BatchStatus::Cancelled;
            }
            chunk_index += 1;
        }
    }

    if ctx.is_stale() {
        return BatchStatus::Cancelled;
    }
    finish_effect_tail(ctx);
    finish_timeline_tail(ctx);
    ctx.flush_overlays();
    if ctx.is_stale() {
        BatchStatus::Cancelled
    } else if ctx.failed() {
        BatchStatus::Failed
    } else {
        BatchStatus::Completed
    }
}

fn synthesize_direct_timeline_chunk(
    chunk: &PreparedSpeechChunk,
    actions: &[TimelineChunkAction],
    requested_acss: Option<&NormalizedAcss>,
    requested_effects: Option<&PostSynthesisStyle>,
    state: &TtsState,
    final_window: bool,
    ctx: &SynthCtx,
) -> bool {
    if ctx.is_stale() {
        return false;
    }
    let descriptor = ctx.engine.descriptor();
    let acss = requested_acss
        .cloned()
        .unwrap_or_default()
        .degrade_for(&descriptor.capabilities.acss);
    let effects = requested_effects
        .cloned()
        .unwrap_or_default()
        .degrade_for(&descriptor.capabilities.post_synthesis_dimensions);
    let mut settings = TtsSettings {
        voice: state.current_voice.clone(),
        rate: state.speech_rate,
        pitch: state.pitch_multiplier,
        volume: 1.0,
    };
    crate::routing::apply_normalized_acss(&mut settings, &acss.style);
    let request = match with_timeline_anchors(
        SynthesisRequest::new(&chunk.text, settings).with_normalized_acss(acss.style.clone()),
        &chunk.capitalization_tones,
        actions,
    ) {
        Ok(request) => attach_synthesis_cancellation(request, ctx),
        Err(error) => {
            ctx.mark_failed();
            warn!("Structured timeline synthesis request error: {error}");
            return true;
        }
    };
    let synthesis_started_at = Instant::now();
    info!(
        lifecycle_stage = "synthesis_started",
        engine_id = %descriptor.id,
        text_bytes = chunk.text.len(),
        "Speech lifecycle started structured synthesis"
    );
    if descriptor_supports_progressive_anchors(
        &descriptor,
        &request.anchors,
        ctx.timeline_renderer.is_some(),
    ) {
        let synthesis = synthesize_direct_chunk_progressively(
            &request,
            &chunk.text,
            None,
            &chunk.capitalization_tones,
            actions,
            effects.style.clone(),
            effects.omitted.clone(),
            state,
            final_window,
            final_window,
            ctx,
        );
        return match synthesis {
            Ok(_) if ctx.is_stale() => false,
            Ok(_) => true,
            Err(error) => {
                warn!(
                    lifecycle_stage = "synthesis_failed",
                    engine_id = %descriptor.id,
                    synthesis_elapsed_us = u64::try_from(
                        synthesis_started_at.elapsed().as_micros()
                    )
                    .unwrap_or(u64::MAX),
                    error = %error,
                    "Speech lifecycle failed direct progressive structured synthesis"
                );
                ctx.mark_failed();
                true
            }
        };
    }
    let synthesis = ctx.engine.synthesize(&request).and_then(|mut result| {
        result.resolve_anchors(&request, descriptor.capabilities.markers.requested_anchors);
        result.degraded_acss = acss.omitted.clone();
        result.validate(&request)?;
        Ok(result)
    });
    match synthesis {
        Ok(result) => {
            info!(
                lifecycle_stage = "synthesis_completed",
                engine_id = %descriptor.id,
                frames = result.audio.frame_count(),
                synthesis_elapsed_us = u64::try_from(
                    synthesis_started_at.elapsed().as_micros()
                )
                .unwrap_or(u64::MAX),
                "Speech lifecycle completed structured synthesis"
            );
            if ctx.is_stale() {
                return false;
            }
            queue_synthesis_result(
                result,
                &chunk.text,
                None,
                &chunk.capitalization_tones,
                actions,
                &effects.style,
                &effects.omitted,
                state,
                final_window,
                final_window,
                ctx,
            );
            !ctx.is_stale()
        }
        Err(error) => {
            warn!(
                lifecycle_stage = "synthesis_failed",
                engine_id = %descriptor.id,
                synthesis_elapsed_us = u64::try_from(
                    synthesis_started_at.elapsed().as_micros()
                )
                .unwrap_or(u64::MAX),
                error = %error,
                "Speech lifecycle failed structured synthesis"
            );
            ctx.mark_failed();
            warn!("Structured timeline synthesis error: {error}");
            true
        }
    }
}

fn prepare_timeline_resources(
    actions: &[PresentationTimelineAction],
    state: &TtsState,
    loader: &AudioFileLoader,
    cancelled: &dyn Fn() -> bool,
) -> Result<HashMap<String, TimelineAudioResource>, TimelinePreparationError> {
    prepare_timeline_resources_with_sample_limit(
        actions,
        state,
        loader,
        MAX_PRESENTATION_DECODED_PCM_SAMPLES,
        cancelled,
    )
}

fn prepare_timeline_resources_with_sample_limit(
    actions: &[PresentationTimelineAction],
    state: &TtsState,
    loader: &AudioFileLoader,
    sample_limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<HashMap<String, TimelineAudioResource>, TimelinePreparationError> {
    let mut resources = HashMap::new();
    let mut budget = PresentationPcmBudget::new(sample_limit);
    for action in actions {
        check_timeline_preparation_cancelled(cancelled)?;
        let resource = match &action.action {
            PresentationAction::Audio {
                path,
                pan,
                effect_bus,
                ..
            } => {
                let audio = loader
                    .load_shared(std::path::Path::new(path))
                    .map_err(|error| {
                        TimelinePreparationError::Invalid(format!("action {}: {error}", action.id))
                    })?;
                check_timeline_preparation_cancelled(cancelled)?;
                budget.retain_shared(&action.id, &audio)?;
                if state.sound_volume != 1.0
                    || state.sound_routing.channel_mode != ChannelMode::Both
                    || *pan != 0.5
                    || *effect_bus == PresentationEffectBus::Speech
                {
                    budget.retain_private(
                        &action.id,
                        audio
                            .samples
                            .len()
                            .saturating_add(effect_tail_samples(*effect_bus)),
                    )?;
                }
                if audio.is_empty() {
                    return Err(TimelinePreparationError::Invalid(format!(
                        "action {} decoded to empty audio",
                        action.id
                    )));
                }
                TimelineAudioResource::File { audio, pan: *pan }
            }
            PresentationAction::Tone {
                frequency_hz,
                duration_ms,
                pan,
                effect_bus,
                ..
            } => {
                budget.retain_private(
                    &action.id,
                    canonical_samples_for_duration_ms(*duration_ms)
                        .saturating_add(effect_tail_samples(*effect_bus)),
                )?;
                TimelineAudioResource::Tone {
                    frequency_hz: *frequency_hz,
                    duration_ms: *duration_ms,
                    pan: *pan,
                }
            }
            PresentationAction::Silence { duration_ms } => {
                budget
                    .retain_private(&action.id, canonical_samples_for_duration_ms(*duration_ms))?;
                TimelineAudioResource::Silence {
                    duration_ms: *duration_ms,
                }
            }
            PresentationAction::SemanticEvent => continue,
        };
        resources.insert(action.id.clone(), resource);
    }
    Ok(resources)
}

fn apply_action_pan(audio: &mut AudioBuffer, normalized_pan: f32) {
    let pan = normalized_pan.clamp(0.0, 1.0) * 2.0 - 1.0;
    for frame in audio.samples.chunks_exact_mut(2) {
        if pan < 0.0 {
            frame[1] *= 1.0 + pan;
        } else {
            frame[0] *= 1.0 - pan;
        }
    }
}

fn prepare_timeline_spans(
    timeline: &PresentationTimelineEnvelope,
    state: &TtsState,
    resources: &HashMap<String, TimelineAudioResource>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<PreparedTimelineSpan>, TimelinePreparationError> {
    check_timeline_preparation_cancelled(cancelled)?;
    let actions_by_span = index_timeline_actions(&timeline.actions);
    check_timeline_preparation_cancelled(cancelled)?;
    let mut spans = Vec::with_capacity(timeline.spans.len());
    for span in &timeline.spans {
        check_timeline_preparation_cancelled(cancelled)?;
        spans.push(prepare_timeline_span(
            span,
            actions_by_span.get(&span.id).map_or(&[], Vec::as_slice),
            state,
            resources,
            cancelled,
        )?);
    }
    Ok(spans)
}

/// Validate the operational action limit before a structured presentation is
/// allowed to cancel active work or consume synthesis-queue capacity.
pub(crate) fn validate_presentation_timeline_action_windows(
    timeline: &PresentationTimelineEnvelope,
    state: &TtsState,
) -> Result<(), String> {
    let actions_by_span = index_timeline_actions(&timeline.actions);
    for span in &timeline.spans {
        let Some(actions) = actions_by_span.get(&span.id) else {
            continue;
        };
        let layout = prepare_timeline_span_layout(span, actions, state);
        validate_timeline_span_layout(span.id, &layout)?;
    }
    Ok(())
}

fn index_timeline_actions(
    actions: &[PresentationTimelineAction],
) -> HashMap<u64, Vec<&PresentationTimelineAction>> {
    let mut actions_by_span = HashMap::new();
    for action in actions {
        actions_by_span
            .entry(action.position.span_id())
            .or_insert_with(Vec::new)
            .push(action);
    }
    actions_by_span
}

fn prepare_timeline_span_layout(
    span: &PresentationSpeechSpan,
    actions: &[&PresentationTimelineAction],
    state: &TtsState,
) -> PreparedTimelineSpanLayout {
    let source_offsets = actions
        .iter()
        .filter_map(|action| match action.position {
            PresentationTimelinePosition::TextOffset { utf8_offset, .. } => Some(utf8_offset),
            PresentationTimelinePosition::SpanBoundary { .. } => None,
        })
        .collect::<Vec<_>>();
    let (mut prepared, mapped_offsets) =
        prepare_speech_text_with_offsets(&span.text, state, &source_offsets);
    for (index, tone) in prepared.capitalization_tones.iter_mut().enumerate() {
        tone.id = format!("omnivox.cap.{}.{}", span.id, index);
    }
    let chunks = chunk_prepared_speech(prepared, 15);
    let mut mapped_offset_index = 0_usize;
    let action_positions = actions
        .iter()
        .map(|action| {
            let (prepared_offset, affinity) = match action.position {
                PresentationTimelinePosition::SpanBoundary { affinity, .. } => match affinity {
                    PresentationAffinity::Before => (0, AnchorAffinity::Before),
                    PresentationAffinity::After => (
                        chunks.last().map_or(0, |chunk| chunk.source_end),
                        AnchorAffinity::After,
                    ),
                },
                PresentationTimelinePosition::TextOffset { affinity, .. } => {
                    let offset = mapped_offsets[mapped_offset_index];
                    mapped_offset_index += 1;
                    (
                        offset,
                        match affinity {
                            PresentationAffinity::Before => AnchorAffinity::Before,
                            PresentationAffinity::After => AnchorAffinity::After,
                        },
                    )
                }
            };
            let chunk_index = locate_timeline_chunk(&chunks, prepared_offset, affinity);
            let chunk = &chunks[chunk_index];
            PreparedTimelineActionPosition {
                chunk_index,
                local_offset: prepared_offset.clamp(chunk.source_start, chunk.source_end)
                    - chunk.source_start,
                affinity,
            }
        })
        .collect();
    PreparedTimelineSpanLayout {
        chunks,
        action_positions,
    }
}

fn validate_timeline_span_layout(
    span_id: u64,
    layout: &PreparedTimelineSpanLayout,
) -> Result<(), String> {
    let max_window_actions = MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW
        .min(MAX_SYNTHESIS_ANCHORS)
        .min(MAX_TIMELINE_ACTIONS_PER_WINDOW);
    let mut action_counts = layout
        .chunks
        .iter()
        .map(|chunk| chunk.capitalization_tones.len())
        .collect::<Vec<_>>();
    for position in &layout.action_positions {
        action_counts[position.chunk_index] += 1;
    }
    for (chunk_index, action_count) in action_counts.into_iter().enumerate() {
        if action_count > max_window_actions {
            return Err(format!(
                "span {span_id} chunk {} contains {action_count} actions including capitalization anchors; maximum is {max_window_actions}",
                chunk_index + 1
            ));
        }
    }
    Ok(())
}

fn prepare_timeline_span(
    span: &PresentationSpeechSpan,
    span_actions: &[&PresentationTimelineAction],
    state: &TtsState,
    resources: &HashMap<String, TimelineAudioResource>,
    cancelled: &dyn Fn() -> bool,
) -> Result<PreparedTimelineSpan, TimelinePreparationError> {
    check_timeline_preparation_cancelled(cancelled)?;
    let mut acss = span.acss.clone();
    if let Some(rate_offset) = span.rate_offset.filter(|offset| *offset != 0) {
        acss.rate = Some(apply_rate_offset(state.speech_rate, rate_offset));
    }
    check_timeline_preparation_cancelled(cancelled)?;
    let layout = prepare_timeline_span_layout(span, span_actions, state);
    check_timeline_preparation_cancelled(cancelled)?;
    validate_timeline_span_layout(span.id, &layout).map_err(TimelinePreparationError::Invalid)?;
    let PreparedTimelineSpanLayout {
        chunks,
        action_positions,
    } = layout;
    let mut actions_by_chunk = vec![Vec::new(); chunks.len()];
    for (action, position) in span_actions.iter().copied().zip(action_positions) {
        check_timeline_preparation_cancelled(cancelled)?;
        let kind = match &action.action {
            PresentationAction::Audio {
                mode,
                volume,
                effect_bus,
                ..
            }
            | PresentationAction::Tone {
                mode,
                volume,
                effect_bus,
                ..
            } => TimelineChunkActionKind::Audio {
                resource: resources.get(&action.id).cloned().ok_or_else(|| {
                    TimelinePreparationError::Invalid(format!(
                        "action {} has no prepared resource",
                        action.id
                    ))
                })?,
                mode: convert_audio_mode(*mode),
                volume: *volume,
                effect_bus: convert_effect_bus(*effect_bus),
            },
            PresentationAction::Silence { .. } => TimelineChunkActionKind::Audio {
                resource: resources.get(&action.id).cloned().ok_or_else(|| {
                    TimelinePreparationError::Invalid(format!(
                        "action {} has no prepared silence",
                        action.id
                    ))
                })?,
                mode: AudioActionMode::Insert,
                volume: 1.0,
                effect_bus: EffectBus::Dry,
            },
            PresentationAction::SemanticEvent => TimelineChunkActionKind::SemanticEvent,
        };
        actions_by_chunk[position.chunk_index].push(TimelineChunkAction {
            id: action.id.clone(),
            text_offset: position.local_offset,
            affinity: position.affinity,
            kind,
        });
    }
    Ok(PreparedTimelineSpan {
        id: span.id,
        logical_voice_id: span.logical_voice_id.clone(),
        acss,
        effects: span.effects.clone(),
        chunks,
        actions: actions_by_chunk,
    })
}

fn locate_timeline_chunk(
    chunks: &[PreparedSpeechChunk],
    offset: u32,
    affinity: AnchorAffinity,
) -> usize {
    match affinity {
        AnchorAffinity::Before => chunks
            .partition_point(|chunk| chunk.source_end <= offset)
            .min(chunks.len() - 1),
        AnchorAffinity::After => chunks
            .partition_point(|chunk| chunk.source_start < offset)
            .saturating_sub(1),
    }
}

fn convert_audio_mode(mode: PresentationAudioMode) -> AudioActionMode {
    match mode {
        PresentationAudioMode::Insert => AudioActionMode::Insert,
        PresentationAudioMode::Overlay => AudioActionMode::Overlay,
    }
}

fn convert_effect_bus(bus: PresentationEffectBus) -> EffectBus {
    match bus {
        PresentationEffectBus::Dry => EffectBus::Dry,
        PresentationEffectBus::Speech => EffectBus::Speech,
    }
}

fn acss_has_values(style: &NormalizedAcss) -> bool {
    style.rate.is_some()
        || style.average_pitch.is_some()
        || style.pitch_range.is_some()
        || style.stress.is_some()
        || style.richness.is_some()
        || style.volume.is_some()
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
    state.current_voice = legacy_voice_for_engine(ctx.engine, &state.current_voice);

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
    let mut logical_route =
        initial_legacy_route(&state, ctx, &mut logical_voice_routing, engine_registry);
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
                            &[],
                            None,
                            None,
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
                    state.current_voice = legacy_voice_for_engine(ctx.engine, &voice);
                    logical_route = initial_legacy_route(
                        &state,
                        ctx,
                        &mut logical_voice_routing,
                        engine_registry,
                    );
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
                            logical_route = initial_legacy_route(
                                &state,
                                ctx,
                                &mut logical_voice_routing,
                                engine_registry,
                            );
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
                placement,
            } => match prepare_tone_audio(frequency, duration, &state) {
                Ok(buf) => match tone_queue_target(placement) {
                    ToneQueueTarget::Stream(stream) => ctx.queue(stream, &buf),
                    ToneQueueTarget::Overlay => ctx.queue_overlay(buf),
                },
                Err(error) => {
                    ctx.mark_failed();
                    warn!("Tone pipeline error: {error}");
                }
            },

            QueueItem::Silence { duration } => {
                let buf = AudioBuffer::silence(duration as f32 / 1000.0);
                let final_timeline_window = primary_window_index + 1 == total_primary_windows;
                let effects = logical_route
                    .as_ref()
                    .map(|route| route.effects.style.clone())
                    .unwrap_or_default();
                match render_primary_window(&buf, &effects, final_timeline_window, ctx) {
                    Ok((rendered, tails)) => {
                        ctx.queue(StreamType::Speech, &rendered);
                        for tail in tails {
                            ctx.queue_overlay(tail);
                        }
                    }
                    Err(error) => {
                        ctx.mark_failed();
                        warn!(
                            "Silence timeline render error; queueing dry silence: {}",
                            error
                        );
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

    if ctx.is_stale() {
        return BatchStatus::Cancelled;
    }
    finish_effect_tail(ctx);
    finish_timeline_tail(ctx);
    ctx.flush_overlays();

    if ctx.is_stale() {
        BatchStatus::Cancelled
    } else if ctx.failed() {
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
    use omnivox_audio::{AudioBackend, AudioStreams, PlaybackStatus};
    use omnivox_core::state::CapitalizationPresentation;

    struct PipelineTestEngine;

    impl TtsEngine for PipelineTestEngine {
        fn descriptor(&self) -> omnivox_tts::contracts::EngineDescriptor {
            panic!("pipeline sink test does not inspect its context engine")
        }

        fn synthesize(&self, _request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            Err(TtsError::NotAvailable)
        }

        fn stop(&self) {}

        fn is_speaking(&self) -> bool {
            false
        }

        fn available_voices(&self) -> Vec<omnivox_tts::VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<omnivox_tts::VoiceInfo> {
            None
        }
    }

    const CAPITAL_TONE_HZ: f32 = 440.0;
    const CAPITAL_TONE_DURATION_MS: u32 = 20;

    fn result(audio: AudioBuffer) -> SynthesisResult {
        SynthesisResult::audio("mock", None, audio)
    }

    #[test]
    fn test_canonicalize_synthesis_result() {
        let tts_buf = omnivox_tts::AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]);
        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;
        assert_eq!(audio_buf.samples, vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(audio_buf.frame_count(), 2);
    }

    #[test]
    fn isolated_capital_uses_pitch_without_presentation_actions() {
        let mut state = TtsState::default();
        for presentation in [
            CapitalizationPresentation::None,
            CapitalizationPresentation::Spoken,
            CapitalizationPresentation::Tone,
            CapitalizationPresentation::SpokenTone,
            CapitalizationPresentation::Custom,
        ] {
            state.capitalization_presentation = presentation;
            state.pitch_multiplier = 0.8;
            assert_eq!(prepare_isolated_letter("A", &mut state), "a");
            assert_eq!(state.pitch_multiplier, ISOLATED_CAPITAL_PITCH_MULTIPLIER);
        }

        state.pitch_multiplier = 0.8;
        assert_eq!(prepare_isolated_letter("a", &mut state), "a");
        assert_eq!(state.pitch_multiplier, 0.8);
    }

    #[test]
    fn test_canonicalize_synthesis_result_empty() {
        let tts_buf = omnivox_tts::AudioBuffer::empty();
        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;
        assert!(audio_buf.is_empty());
    }

    #[test]
    fn progressive_timeline_renders_an_insert_resolved_at_a_window_boundary() {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let generation = AtomicU64::new(1);
        let lifecycle = RequestLifecycle::default();
        let engine = PipelineTestEngine;
        let state = TtsState::default();
        let tickets = Mutex::new(Vec::new());
        let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
        let effect_processor = Mutex::new(PostSynthesisProcessor::new());
        let ctx = SynthCtx {
            gen: 1,
            gen_counter: &generation,
            cancellation: None,
            lifecycle: &lifecycle,
            engine: &engine,
            control: &control,
            playback_tickets: Some(&tickets),
            presentation_clock: None,
            pending_overlays: None,
            timeline_renderer: Some(&timeline_renderer),
            effect_processor: Some(&effect_processor),
            marker_dispatch: None,
            batch_failed: None,
        };
        let actions = vec![TimelineChunkAction {
            id: "pause".to_owned(),
            text_offset: 4,
            affinity: AnchorAffinity::After,
            kind: TimelineChunkActionKind::Audio {
                resource: TimelineAudioResource::Silence { duration_ms: 1 },
                mode: AudioActionMode::Insert,
                volume: 1.0,
                effect_bus: EffectBus::Dry,
            },
        }];
        let mut sink = ProgressiveChunkSink::new(
            "test",
            None,
            PostSynthesisStyle::default(),
            Vec::new(),
            &state,
            false,
            true,
            &[],
            &actions,
            &ctx,
        )
        .unwrap();
        sink.start(SynthesisStreamStart {
            engine_id: "exact".to_owned(),
            actual_voice: None,
            degraded_acss: Vec::new(),
        })
        .unwrap();
        sink.audio(AudioBuffer::new(vec![0.25; 8])).unwrap();
        assert_eq!(sink.primary_frame_count, 4);
        assert_eq!(sink.output_frame_count, 4);
        sink.markers(
            Vec::new(),
            vec![ResolvedAnchor {
                id: "pause".to_owned(),
                frame_offset: Some(4),
                resolution: AnchorResolution::WordBoundary,
            }],
        )
        .unwrap();

        sink.finish(SynthesisStreamCompletion { frame_count: 4 })
            .unwrap();

        assert_eq!(sink.primary_frame_count, 4);
        assert_eq!(sink.output_frame_count, 48);
        assert!(sink.resolved_timeline_actions.is_empty());
        let ticket = tickets.lock().unwrap()[0].clone();
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        control.drain();
    }

    #[test]
    fn progressive_timeline_rejects_an_omitted_anchor() {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let generation = AtomicU64::new(1);
        let lifecycle = RequestLifecycle::default();
        let engine = PipelineTestEngine;
        let state = TtsState::default();
        let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
        let effect_processor = Mutex::new(PostSynthesisProcessor::new());
        let ctx = SynthCtx {
            gen: 1,
            gen_counter: &generation,
            cancellation: None,
            lifecycle: &lifecycle,
            engine: &engine,
            control: &control,
            playback_tickets: None,
            presentation_clock: None,
            pending_overlays: None,
            timeline_renderer: Some(&timeline_renderer),
            effect_processor: Some(&effect_processor),
            marker_dispatch: None,
            batch_failed: None,
        };
        let tones = vec![CapitalizationTone {
            id: "capital".to_owned(),
            text_offset: 0,
            frequency_hz: CAPITAL_TONE_HZ,
            duration_ms: CAPITAL_TONE_DURATION_MS,
        }];
        let mut sink = ProgressiveChunkSink::new(
            "A",
            None,
            PostSynthesisStyle::default(),
            Vec::new(),
            &state,
            true,
            true,
            &tones,
            &[],
            &ctx,
        )
        .unwrap();
        sink.start(SynthesisStreamStart {
            engine_id: "exact".to_owned(),
            actual_voice: None,
            degraded_acss: Vec::new(),
        })
        .unwrap();

        let error = match sink.finish(SynthesisStreamCompletion { frame_count: 0 }) {
            Ok(_) => panic!("missing timeline anchor unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("resolved 0 of 1 timeline anchors"));
    }

    #[test]
    fn canonicalization_preserves_metadata() {
        let mut result = SynthesisResult::new(
            "helper",
            Some(PhysicalVoiceId::new("helper", "voice")),
            AudioBuffer::new(vec![0.1; 1026]),
            vec![SynthesisMarker {
                kind: omnivox_tts::SynthesisMarkerKind::Word,
                frame_offset: 400,
                text_start: Some(0),
                text_length: Some(5),
                value: None,
            }],
        );
        result.anchors.push(omnivox_tts::ResolvedAnchor {
            id: "cue".to_owned(),
            frame_offset: Some(400),
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
    fn presentation_tone_modes_select_the_required_playback_clock() {
        assert_eq!(
            tone_queue_target(TonePlacement::Independent),
            ToneQueueTarget::Stream(StreamType::Tone)
        );
        assert_eq!(
            tone_queue_target(TonePlacement::Insert),
            ToneQueueTarget::Stream(StreamType::Speech)
        );
        assert_eq!(
            tone_queue_target(TonePlacement::Overlay),
            ToneQueueTarget::Overlay
        );
    }

    fn boundary_action(id: &str, action: PresentationAction) -> PresentationTimelineAction {
        PresentationTimelineAction {
            id: id.to_owned(),
            position: PresentationTimelinePosition::SpanBoundary {
                span_id: 1,
                affinity: PresentationAffinity::After,
            },
            lifecycle_anchor: omnivox_tts::timeline_protocol::PresentationLifecycleAnchor::Run,
            action,
        }
    }

    fn never_cancelled() -> bool {
        false
    }

    fn semantic_chunk_actions(count: usize) -> Vec<TimelineChunkAction> {
        (0..count)
            .map(|index| TimelineChunkAction {
                id: format!("semantic.{index}"),
                text_offset: 0,
                affinity: AnchorAffinity::After,
                kind: TimelineChunkActionKind::SemanticEvent,
            })
            .collect()
    }

    fn semantic_timeline_actions(count: usize) -> Vec<PresentationTimelineAction> {
        (0..count)
            .map(|index| {
                boundary_action(
                    &format!("semantic.{index}"),
                    PresentationAction::SemanticEvent,
                )
            })
            .collect()
    }

    #[test]
    fn direct_timeline_anchor_limit_returns_an_error_instead_of_panicking() {
        let tone = CapitalizationTone {
            id: "capital".to_owned(),
            text_offset: 0,
            frequency_hz: CAPITAL_TONE_HZ,
            duration_ms: CAPITAL_TONE_DURATION_MS,
        };
        let accepted = semantic_chunk_actions(MAX_SYNTHESIS_ANCHORS - 1);
        let request = with_timeline_anchors(
            SynthesisRequest::new("test", TtsSettings::default()),
            std::slice::from_ref(&tone),
            &accepted,
        )
        .unwrap();
        assert_eq!(request.anchors.len(), MAX_SYNTHESIS_ANCHORS);

        let overflow = semantic_chunk_actions(MAX_SYNTHESIS_ANCHORS);
        let error = with_timeline_anchors(
            SynthesisRequest::new("test", TtsSettings::default()),
            &[tone],
            &overflow,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            omnivox_tts::TtsError::InvalidParameter(message)
                if message.contains("anchor limit")
        ));
    }

    #[test]
    fn prepared_timeline_window_enforces_the_downstream_action_limit() {
        let span = PresentationSpeechSpan {
            id: 1,
            text: "test".to_owned(),
            logical_voice_id: None,
            acss: NormalizedAcss::default(),
            rate_offset: None,
            effects: PresentationEffectDirective::Retain,
        };
        let accepted = semantic_timeline_actions(MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW);
        let accepted = accepted.iter().collect::<Vec<_>>();
        let prepared = prepare_timeline_span(
            &span,
            &accepted,
            &TtsState::default(),
            &HashMap::new(),
            &never_cancelled,
        )
        .unwrap();
        assert_eq!(
            prepared.actions[0].len(),
            MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW
        );

        let overflow = semantic_timeline_actions(MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW + 1);
        let overflow = overflow.iter().collect::<Vec<_>>();
        let error = prepare_timeline_span(
            &span,
            &overflow,
            &TtsState::default(),
            &HashMap::new(),
            &never_cancelled,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("513 actions"));
        assert!(error.contains("maximum is 512"));
    }

    #[test]
    fn prepared_timeline_action_limit_is_per_speech_window() {
        let text = (1..=30)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let second_window = text.find("word16").unwrap() as u32;
        let span = PresentationSpeechSpan {
            id: 1,
            text,
            logical_voice_id: None,
            acss: NormalizedAcss::default(),
            rate_offset: None,
            effects: PresentationEffectDirective::Retain,
        };
        let mut actions = semantic_timeline_actions(MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW + 1);
        for action in actions
            .iter_mut()
            .take(MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW / 2)
        {
            action.position = PresentationTimelinePosition::SpanBoundary {
                span_id: 1,
                affinity: PresentationAffinity::Before,
            };
        }
        for action in actions
            .iter_mut()
            .skip(MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW / 2)
        {
            action.position = PresentationTimelinePosition::TextOffset {
                span_id: 1,
                utf8_offset: second_window,
                affinity: PresentationAffinity::Before,
            };
        }

        validate_presentation_timeline_action_windows(
            &PresentationTimelineEnvelope {
                protocol_version:
                    omnivox_tts::timeline_protocol::PRESENTATION_TIMELINE_PROTOCOL_VERSION,
                generation: 1,
                dispatch_id: 1,
                delivery_policy: Some(
                    omnivox_tts::timeline_protocol::PresentationDeliveryPolicy::Ordered,
                ),
                replacement_key: None,
                spans: vec![span],
                actions,
            },
            &TtsState::default(),
        )
        .unwrap();
    }

    #[test]
    fn timeline_action_index_groups_once_and_preserves_wire_order() {
        let mut actions = semantic_timeline_actions(4);
        for (action, span_id) in actions.iter_mut().zip([2, 1, 2, 1]) {
            action.position = PresentationTimelinePosition::SpanBoundary {
                span_id,
                affinity: PresentationAffinity::After,
            };
        }

        let indexed = index_timeline_actions(&actions);

        assert_eq!(
            indexed[&1]
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec!["semantic.1", "semantic.3"]
        );
        assert_eq!(
            indexed[&2]
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec!["semantic.0", "semantic.2"]
        );
    }

    #[test]
    fn generated_timeline_resources_remain_deferred_recipes() {
        let actions = [
            boundary_action(
                "tone",
                PresentationAction::Tone {
                    frequency_hz: 440.0,
                    duration_ms: 50,
                    mode: PresentationAudioMode::Insert,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            ),
            boundary_action("silence", PresentationAction::Silence { duration_ms: 100 }),
        ];

        let resources = prepare_timeline_resources(
            &actions,
            &TtsState::default(),
            &AudioFileLoader::new(),
            &never_cancelled,
        )
        .unwrap();

        assert!(matches!(
            resources.get("tone"),
            Some(TimelineAudioResource::Tone { .. })
        ));
        assert!(matches!(
            resources.get("silence"),
            Some(TimelineAudioResource::Silence { .. })
        ));
    }

    #[test]
    fn presentation_pcm_budget_reserves_deferred_generated_resources() {
        let tone = |id: &str| {
            boundary_action(
                id,
                PresentationAction::Tone {
                    frequency_hz: 440.0,
                    duration_ms: 50,
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            )
        };

        let error = prepare_timeline_resources_with_sample_limit(
            &[tone("tone.1"), tone("tone.2")],
            &TtsState::default(),
            &AudioFileLoader::new(),
            canonical_samples_for_duration_ms(50),
            &never_cancelled,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("action tone.2"));
        assert!(error.contains("presentation maximum"));
    }

    #[test]
    fn missing_file_resource_still_rejects_atomic_preparation() {
        let actions = [
            boundary_action(
                "tone",
                PresentationAction::Tone {
                    frequency_hz: 440.0,
                    duration_ms: 50,
                    mode: PresentationAudioMode::Insert,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            ),
            boundary_action(
                "missing",
                PresentationAction::Audio {
                    path: format!(
                        "/tmp/omnivox-missing-timeline-resource-{}",
                        std::process::id()
                    ),
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            ),
        ];

        let error = prepare_timeline_resources(
            &actions,
            &TtsState::default(),
            &AudioFileLoader::new(),
            &never_cancelled,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("action missing"));
        assert!(error.contains("File not found"));
    }

    #[test]
    fn default_file_resource_reuses_loader_cache_pcm() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-sounds/complete.ogg");
        let loader = AudioFileLoader::with_cache();
        let cached = loader.load_shared(&path).unwrap();
        let resources = prepare_timeline_resources(
            &[boundary_action(
                "file",
                PresentationAction::Audio {
                    path: path.to_string_lossy().into_owned(),
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            )],
            &TtsState::default(),
            &loader,
            &never_cancelled,
        )
        .unwrap();

        let TimelineAudioResource::File {
            audio: prepared, ..
        } = resources.get("file").unwrap()
        else {
            panic!("file resource was not prepared eagerly");
        };
        assert!(Arc::ptr_eq(&cached, prepared));
    }

    #[test]
    fn file_resource_processing_is_deferred_to_its_render_window() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-sounds/complete.ogg");
        let loader = AudioFileLoader::with_cache();
        let cached = loader.load_shared(&path).unwrap();
        let original = cached.samples.clone();
        let mut expected = (*cached).clone();
        let mut state = TtsState {
            sound_volume: 0.5,
            ..TtsState::default()
        };
        state.sound_routing.channel_mode = ChannelMode::Left;
        build_sound_pipeline(&state).process(&mut expected).unwrap();
        apply_action_pan(&mut expected, 0.25);

        let resources = prepare_timeline_resources(
            &[boundary_action(
                "file",
                PresentationAction::Audio {
                    path: path.to_string_lossy().into_owned(),
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.25,
                    effect_bus: PresentationEffectBus::Dry,
                },
            )],
            &state,
            &loader,
            &never_cancelled,
        )
        .unwrap();

        let TimelineAudioResource::File {
            audio: prepared, ..
        } = resources.get("file").unwrap()
        else {
            panic!("file resource was not prepared eagerly");
        };
        assert!(Arc::ptr_eq(&cached, prepared));
        let materialized = resources.get("file").unwrap().materialize(&state).unwrap();
        assert!(!Arc::ptr_eq(&cached, &materialized));
        assert_eq!(materialized.samples, expected.samples);
        assert_eq!(cached.samples, original);
        assert!(Arc::ptr_eq(&cached, &loader.load_shared(&path).unwrap()));
    }

    #[test]
    fn presentation_pcm_budget_reserves_transforms_but_deduplicates_shared_pcm() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-sounds/complete.ogg");
        let loader = AudioFileLoader::with_cache();
        let cached = loader.load_shared(&path).unwrap();
        let action = |id: &str| {
            boundary_action(
                id,
                PresentationAction::Audio {
                    path: path.to_string_lossy().into_owned(),
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            )
        };
        let shared = prepare_timeline_resources_with_sample_limit(
            &[action("shared.1"), action("shared.2")],
            &TtsState::default(),
            &loader,
            cached.samples.len(),
            &never_cancelled,
        )
        .unwrap();
        for resource in shared.values() {
            let TimelineAudioResource::File { audio, .. } = resource else {
                panic!("file resource was not prepared eagerly");
            };
            assert!(Arc::ptr_eq(&cached, audio));
        }

        let state = TtsState {
            sound_volume: 0.5,
            ..TtsState::default()
        };
        let error = prepare_timeline_resources_with_sample_limit(
            &[action("private.1"), action("private.2")],
            &state,
            &loader,
            cached.samples.len() * 2,
            &never_cancelled,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            MAX_PRESENTATION_DECODED_PCM_SAMPLES * std::mem::size_of::<f32>(),
            64 * 1024 * 1024
        );
        assert!(error.contains("action private.2"));
        assert!(error.contains("presentation maximum"));
        assert!(Arc::ptr_eq(&cached, &loader.load_shared(&path).unwrap()));
    }

    #[test]
    fn timeline_resource_preparation_honours_midstream_cancellation() {
        let actions = [
            boundary_action("first", PresentationAction::Silence { duration_ms: 10 }),
            boundary_action("second", PresentationAction::Silence { duration_ms: 10 }),
        ];
        let checks = std::cell::Cell::new(0_usize);
        let cancelled = || {
            let current = checks.get();
            checks.set(current + 1);
            current >= 1
        };

        let error = prepare_timeline_resources(
            &actions,
            &TtsState::default(),
            &AudioFileLoader::new(),
            &cancelled,
        )
        .unwrap_err();

        assert_eq!(error, TimelinePreparationError::Cancelled);
        assert_eq!(checks.get(), 2);
    }

    #[test]
    fn timeline_span_preparation_honours_midstream_cancellation() {
        let span = PresentationSpeechSpan {
            id: 1,
            text: "first".to_owned(),
            logical_voice_id: None,
            acss: NormalizedAcss::default(),
            rate_offset: None,
            effects: PresentationEffectDirective::Retain,
        };
        let timeline = PresentationTimelineEnvelope {
            protocol_version:
                omnivox_tts::timeline_protocol::PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation: 1,
            dispatch_id: 1,
            delivery_policy: None,
            replacement_key: None,
            spans: vec![
                span.clone(),
                PresentationSpeechSpan {
                    id: 2,
                    text: "second".to_owned(),
                    ..span
                },
            ],
            actions: Vec::new(),
        };
        let checks = std::cell::Cell::new(0_usize);
        let cancelled = || {
            let current = checks.get();
            checks.set(current + 1);
            current >= 4
        };

        let error =
            prepare_timeline_spans(&timeline, &TtsState::default(), &HashMap::new(), &cancelled)
                .unwrap_err();

        assert_eq!(error, TimelinePreparationError::Cancelled);
        assert!(checks.get() > 4);
    }

    #[test]
    fn dry_timeline_resource_stays_shared_through_render_preparation() {
        let shared = Arc::new(AudioBuffer::new(vec![0.25, -0.25]));
        let action = TimelineChunkAction {
            id: "shared".to_owned(),
            text_offset: 0,
            affinity: AnchorAffinity::After,
            kind: TimelineChunkActionKind::Audio {
                resource: TimelineAudioResource::File {
                    audio: Arc::clone(&shared),
                    pan: 0.5,
                },
                mode: AudioActionMode::Overlay,
                volume: 1.0,
                effect_bus: EffectBus::Dry,
            },
        };
        let result = CanonicalSynthesisResult {
            audio: AudioBuffer::silence(0.01),
            engine_id: "mock".to_owned(),
            actual_voice: None,
            markers: Vec::new(),
            anchors: vec![ResolvedAnchor {
                id: "shared".to_owned(),
                frame_offset: Some(441),
                resolution: AnchorResolution::Exact,
            }],
            degraded_acss: Vec::new(),
        };

        let (_, resources) = prepare_speech_timeline(
            &result,
            &[],
            &[action],
            &PostSynthesisStyle::default(),
            &TtsState::default(),
        )
        .unwrap();

        assert!(Arc::ptr_eq(&shared, &resources[0].audio));
    }

    #[test]
    fn tone_gain_is_applied_once_across_legacy_structured_and_capital_paths() {
        let frequency_hz = 440.0;
        let duration_ms = 50;
        let unity = ToneGenerator::generate(frequency_hz, duration_ms, 1.0);
        let unity_peak = unity
            .samples
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0, f32::max);

        for volume in [0.0, 0.1, 0.5, 1.0] {
            let state = TtsState {
                tone_volume: volume,
                ..TtsState::default()
            };
            let legacy = prepare_tone_audio(frequency_hz, duration_ms, &state).unwrap();

            let capital_result = CanonicalSynthesisResult {
                audio: AudioBuffer::silence(0.1),
                engine_id: "mock".to_owned(),
                actual_voice: None,
                markers: Vec::new(),
                anchors: vec![ResolvedAnchor {
                    id: "capital".to_owned(),
                    frame_offset: Some(0),
                    resolution: AnchorResolution::Exact,
                }],
                degraded_acss: Vec::new(),
            };
            let (_, capital_resources) = prepare_speech_timeline(
                &capital_result,
                &[CapitalizationTone {
                    id: "capital".to_owned(),
                    text_offset: 0,
                    frequency_hz,
                    duration_ms,
                }],
                &[],
                &PostSynthesisStyle::default(),
                &state,
            )
            .unwrap();

            let structured = prepare_timeline_resources(
                &[PresentationTimelineAction {
                    id: "structured".to_owned(),
                    position: PresentationTimelinePosition::SpanBoundary {
                        span_id: 1,
                        affinity: PresentationAffinity::Before,
                    },
                    lifecycle_anchor:
                        omnivox_tts::timeline_protocol::PresentationLifecycleAnchor::Run,
                    action: PresentationAction::Tone {
                        frequency_hz,
                        duration_ms,
                        mode: PresentationAudioMode::Insert,
                        volume: 1.0,
                        pan: 0.5,
                        effect_bus: PresentationEffectBus::Dry,
                    },
                }],
                &state,
                &AudioFileLoader::new(),
                &never_cancelled,
            )
            .unwrap();
            let structured_audio = structured
                .get("structured")
                .unwrap()
                .materialize(&state)
                .unwrap();

            let expected_peak = unity_peak * volume;
            for (path, audio) in [
                ("legacy", &legacy),
                ("capital", capital_resources[0].audio.as_ref()),
                ("structured", structured_audio.as_ref()),
            ] {
                let actual_peak = audio
                    .samples
                    .iter()
                    .copied()
                    .map(f32::abs)
                    .fold(0.0, f32::max);
                assert!(
                    (actual_peak - expected_peak).abs() < 0.000_001,
                    "{path} peak {actual_peak} did not match one {volume} gain application ({expected_peak})"
                );
            }
        }
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
    fn normalized_effects_map_to_documented_dsp_ranges() {
        let parameters = post_synthesis_parameters(&PostSynthesisStyle {
            gain: Some(0.5),
            low_pass: Some(0.0),
            high_pass: Some(1.0),
            pan: Some(0.25),
            chorus: Some(0.8),
            reverb: Some(0.4),
            echo: Some(0.6),
        });

        assert!((parameters.gain - 1.0).abs() < 0.000_001);
        assert_eq!(parameters.low_pass_hz, Some(200.0));
        assert!((parameters.high_pass_hz.unwrap() - 3_000.0).abs() < 0.001);
        assert_eq!(parameters.pan, -0.5);
        assert_eq!(parameters.chorus, 0.8);
        assert_eq!(parameters.reverb, 0.4);
        assert_eq!(parameters.echo, 0.6);

        let neutral = post_synthesis_parameters(&PostSynthesisStyle::default());
        assert_eq!(neutral, PostSynthesisParameters::default());
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

        let (timeline, resources) = prepare_speech_timeline(
            &result,
            &tones,
            &[],
            &PostSynthesisStyle::default(),
            &TtsState::default(),
        )
        .unwrap();
        let rendered = TimelineAudioRenderer::new()
            .render_shared_window(&result.audio, &timeline, &resources, true)
            .unwrap();

        assert_eq!(timeline.actions[0].output_frame, 10);
        assert_eq!(rendered.audio.frame_count(), 44100);
        assert!(rendered.audio.samples[..20]
            .iter()
            .all(|sample| *sample == 0.0));
        assert!(rendered.audio.samples[20..]
            .iter()
            .any(|sample| *sample != 0.0));
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

        let (timeline, resources) = prepare_speech_timeline(
            &result,
            &tones,
            &[],
            &PostSynthesisStyle::default(),
            &TtsState::default(),
        )
        .unwrap();
        let rendered = TimelineAudioRenderer::new()
            .render_shared_window(&result.audio, &timeline, &resources, true)
            .unwrap();

        assert_eq!(timeline.actions[0].output_frame, 0);
        assert!(rendered.audio.samples.iter().any(|sample| *sample != 0.0));
    }

    #[test]
    fn structured_actions_follow_preprocessed_offsets_across_chunks() {
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen";
        let span = PresentationSpeechSpan {
            id: 7,
            text: text.to_owned(),
            logical_voice_id: Some("comment".to_owned()),
            acss: NormalizedAcss::default(),
            rate_offset: None,
            effects: PresentationEffectDirective::Retain,
        };
        let actions = [
            PresentationTimelineAction {
                id: "opening-cue".to_owned(),
                position: PresentationTimelinePosition::SpanBoundary {
                    span_id: 7,
                    affinity: PresentationAffinity::Before,
                },
                lifecycle_anchor:
                    omnivox_tts::timeline_protocol::PresentationLifecycleAnchor::Object,
                action: PresentationAction::Audio {
                    path: "/unused/test.ogg".to_owned(),
                    mode: PresentationAudioMode::Overlay,
                    volume: 1.0,
                    pan: 0.5,
                    effect_bus: PresentationEffectBus::Dry,
                },
            },
            PresentationTimelineAction {
                id: "sixteenth-word".to_owned(),
                position: PresentationTimelinePosition::TextOffset {
                    span_id: 7,
                    utf8_offset: text.find("sixteen").unwrap() as u32,
                    affinity: PresentationAffinity::Before,
                },
                lifecycle_anchor: omnivox_tts::timeline_protocol::PresentationLifecycleAnchor::Run,
                action: PresentationAction::SemanticEvent,
            },
        ];
        let resources = HashMap::from([(
            "opening-cue".to_owned(),
            TimelineAudioResource::File {
                audio: Arc::new(AudioBuffer::silence(0.01)),
                pan: 0.5,
            },
        )]);
        let actions = actions.iter().collect::<Vec<_>>();

        let prepared = prepare_timeline_span(
            &span,
            &actions,
            &TtsState::default(),
            &resources,
            &never_cancelled,
        )
        .unwrap();

        assert_eq!(prepared.chunks.len(), 2);
        assert_eq!(prepared.actions[0][0].id, "opening-cue");
        assert_eq!(prepared.actions[0][0].text_offset, 0);
        assert_eq!(prepared.actions[1][0].id, "sixteenth-word");
        assert_eq!(prepared.actions[1][0].text_offset, 0);
    }

    #[test]
    fn structured_offset_mapping_handles_reverse_maximum_action_offsets() {
        let text = "aB ".repeat(20_000);
        let mut offsets = (0..MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW * 8)
            .map(|index| (index * text.len() / (MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW * 8)) as u32)
            .rev()
            .collect::<Vec<_>>();
        offsets[1] = offsets[0];
        let state = TtsState {
            punctuation_level: omnivox_core::PunctuationLevel::All,
            split_caps: true,
            ..TtsState::default()
        };

        let (prepared, mapped) = prepare_speech_text_with_offsets(&text, &state, &offsets);

        assert_eq!(mapped.len(), offsets.len());
        assert_eq!(mapped[0], mapped[1]);
        assert!(mapped.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(mapped.iter().all(|offset| {
            let offset = *offset as usize;
            offset <= prepared.text.len() && prepared.text.is_char_boundary(offset)
        }));
    }

    #[test]
    fn structured_audio_pan_uses_normalized_stereo_position() {
        let mut left = AudioBuffer::new(vec![1.0, 1.0, 0.5, 0.5]);
        apply_action_pan(&mut left, 0.25);
        assert_eq!(left.samples, vec![1.0, 0.5, 0.5, 0.25]);

        let mut center = AudioBuffer::new(vec![1.0, -1.0]);
        apply_action_pan(&mut center, 0.5);
        assert_eq!(center.samples, vec![1.0, -1.0]);
    }
}
