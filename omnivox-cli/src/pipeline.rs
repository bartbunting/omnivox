//! Audio synthesis pipeline: buffer conversion, pipeline construction, chunk synthesis.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, AudioPipeline, ChannelRouter, PlaybackTicket,
    PostSynthesisParameters, PostSynthesisProcessor, PreparedAudioResource, SilenceTrimReport,
    SilenceTrimmer, StreamType, TimelineAudioRenderer, ToneGenerator, VolumeAdjust,
};
use omnivox_core::timeline::{
    ActionAffinity, AudioActionMode, EffectBus, PresentationPosition, ResolvedTimelineAction,
    ScheduledTimeline, TimelineAction, TimelineActionId, TimelineActionKind,
};
use omnivox_core::{QueueItem, TonePlacement, TtsState};
use omnivox_tts::contracts::{
    apply_rate_offset, AcssDimension, NormalizedAcss, PhysicalVoiceId, PostSynthesisDimension,
    PostSynthesisStyle,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::timeline_protocol::{
    PresentationAction, PresentationAffinity, PresentationAudioMode, PresentationEffectBus,
    PresentationEffectDirective, PresentationSpeechSpan, PresentationTimelineAction,
    PresentationTimelineEnvelope, PresentationTimelinePosition,
};
use omnivox_tts::{
    AnchorAffinity, AnchorResolution, RequestedAnchor, ResolvedAnchor, SynthesisMarker,
    SynthesisRequest, SynthesisResult, TtsEngine, TtsSettings,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, warn};

use crate::health::RuntimeEngineHealth;
use crate::marker_events::{
    MarkerDispatchContext, PlaybackSemanticEvent, PlaybackTimelineResolution,
};
use crate::routing::{
    legacy_voice_for_engine, LogicalRoute, LogicalVoiceRoutingSnapshot, RuntimeSynthesisOutcome,
};
use crate::text::{
    chunk_prepared_speech, extract_logical_voice, extract_pitch, extract_voice,
    prepare_speech_text, prepare_speech_text_with_offsets, rate_scaled_padding, CapitalizationTone,
    PreparedSpeechChunk, CAPITAL_TONE_DURATION_MS, CAPITAL_TONE_HZ,
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
            self.control
                .queue_tracked_if(stream, buffer, || !self.is_stale())
                .map(|ticket| {
                    if let Some(ticket) = ticket {
                        self.record_ticket(stream, ticket);
                    }
                })
        } else {
            self.control
                .queue_if(stream, buffer, || !self.is_stale())
                .map(|_| ())
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
        match self
            .control
            .queue_overlay_after_if(&buffer, barriers, || !self.is_stale())
        {
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
        audio: AudioBuffer,
        mode: AudioActionMode,
        volume: f32,
        effect_bus: EffectBus,
    },
    SemanticEvent,
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

    let request = SynthesisRequest::new(chunk, settings.clone())
        .with_anchors(requested_timeline_anchors(capitalization_tones, &[]))
        .expect("prepared capitalization offsets are valid");
    match ctx.engine.synthesize(&request).and_then(|mut result| {
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
        match prepared.queue_if(ctx.control, &result.audio, || !ctx.is_stale()) {
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
    let rendered = renderer.lock().unwrap().render_window(
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
) -> Result<(ScheduledTimeline, Vec<PreparedAudioResource>), omnivox_audio::AudioError> {
    let mut actions = Vec::with_capacity(tones.len() + timeline_actions.len());
    let mut resources = Vec::with_capacity(tones.len() + timeline_actions.len());
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
        actions.push(ResolvedTimelineAction {
            action,
            source_frame: frame_offset,
        });
        resources.push(PreparedAudioResource::new(id, audio));
    }
    for action in timeline_actions {
        let resolved = result
            .anchors
            .iter()
            .find(|anchor| anchor.id == action.id)
            .expect("validated synthesis result contains every requested anchor");
        let frame_offset = resolved.frame_offset.unwrap_or_else(|| {
            if action.affinity == AnchorAffinity::After {
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
                "structured timeline placement degraded"
            );
        }
        let id = TimelineActionId::new(action.id.clone())
            .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
        let kind = match &action.kind {
            TimelineChunkActionKind::Audio {
                audio,
                mode,
                volume,
                effect_bus,
            } => {
                let mut audio = audio.clone();
                if *effect_bus == EffectBus::Speech {
                    let mut processor = PostSynthesisProcessor::new();
                    let processed =
                        processor.process_window(&audio, post_synthesis_parameters(effects), true);
                    audio = processed.audio;
                    if let Some(tail) = processed.tail {
                        audio.append(&tail);
                    }
                }
                let duration_frames = audio.frame_count() as u64;
                resources.push(PreparedAudioResource::new(id.clone(), audio));
                TimelineActionKind::Audio {
                    mode: *mode,
                    duration_frames,
                    volume: *volume,
                    effect_bus: *effect_bus,
                }
            }
            TimelineChunkActionKind::SemanticEvent => TimelineActionKind::SemanticEvent,
        };
        actions.push(ResolvedTimelineAction {
            action: TimelineAction {
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
            },
            source_frame: frame_offset,
        });
    }
    let timeline = ScheduledTimeline::build(result.audio.frame_count() as u64, actions)
        .map_err(|error| omnivox_audio::AudioError::TimelineError(error.to_string()))?;
    Ok((timeline, resources))
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

const CAPITAL_ANNOTATION_LOGICAL_VOICE: &str = "annotate";

#[derive(Debug, Clone, PartialEq)]
struct PreparedLetterChunk {
    text: String,
    capitalization_tones: Vec<CapitalizationTone>,
    logical_voice_id: Option<&'static str>,
}

fn prepare_letter_presentation(text: &str, state: &TtsState) -> Vec<PreparedLetterChunk> {
    let is_upper = text.chars().next().is_some_and(char::is_uppercase);
    let capitalization_tones = if is_upper && state.capitalization_presentation.includes_tone() {
        vec![CapitalizationTone {
            id: "capitalization-letter".to_owned(),
            text_offset: 0,
            frequency_hz: CAPITAL_TONE_HZ,
            duration_ms: CAPITAL_TONE_DURATION_MS,
        }]
    } else {
        Vec::new()
    };
    let lowered = text
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let mut chunks = Vec::with_capacity(2);
    if is_upper && state.capitalization_presentation.includes_spoken() {
        chunks.push(PreparedLetterChunk {
            text: "cap".to_owned(),
            capitalization_tones: Vec::new(),
            logical_voice_id: Some(CAPITAL_ANNOTATION_LOGICAL_VOICE),
        });
    }
    chunks.push(PreparedLetterChunk {
        text: lowered,
        capitalization_tones,
        logical_voice_id: None,
    });
    chunks
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
    let chunks = prepare_letter_presentation(text, &state);
    let status = if let Some(mut content_route) =
        initial_legacy_route(&state, ctx, &mut routing, engine_registry)
    {
        let mut status = BatchStatus::Completed;
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.iter().enumerate() {
            let is_last = index + 1 == chunk_count;
            let outcome = if let Some(logical_voice_id) = chunk.logical_voice_id {
                match routing.initial_route(logical_voice_id, engine_registry) {
                    Ok(mut annotation_route) => synthesize_routed_chunk(
                        &chunk.text,
                        &chunk.capitalization_tones,
                        &[],
                        None,
                        None,
                        &state,
                        is_last,
                        is_last,
                        &mut annotation_route,
                        &mut routing,
                        engine_registry,
                        runtime_health,
                        ctx,
                    ),
                    Err(error) => {
                        warn!(
                            "Could not route capital annotation through logical voice {logical_voice_id}: {error}; using the content voice"
                        );
                        synthesize_routed_chunk(
                            &chunk.text,
                            &chunk.capitalization_tones,
                            &[],
                            None,
                            None,
                            &state,
                            is_last,
                            is_last,
                            &mut content_route,
                            &mut routing,
                            engine_registry,
                            runtime_health,
                            ctx,
                        )
                    }
                }
            } else {
                synthesize_routed_chunk(
                    &chunk.text,
                    &chunk.capitalization_tones,
                    &[],
                    None,
                    None,
                    &state,
                    is_last,
                    is_last,
                    &mut content_route,
                    &mut routing,
                    engine_registry,
                    runtime_health,
                    ctx,
                )
            };
            status = match outcome {
                RoutedChunkOutcome::Queued { .. } => BatchStatus::Completed,
                RoutedChunkOutcome::Cancelled => BatchStatus::Cancelled,
                RoutedChunkOutcome::Failed | RoutedChunkOutcome::Exhausted => BatchStatus::Failed,
            };
            if status != BatchStatus::Completed {
                break;
            }
        }
        status
    } else {
        let settings = TtsSettings {
            voice: state.current_voice.clone(),
            rate: state.speech_rate,
            pitch: state.pitch_multiplier,
            volume: 1.0,
        };
        let chunk_count = chunks.len();
        if chunks.iter().enumerate().all(|(index, chunk)| {
            let is_last = index + 1 == chunk_count;
            synthesize_chunk_with_tones(
                &chunk.text,
                &chunk.capitalization_tones,
                &settings,
                &state,
                is_last,
                is_last,
                ctx,
            )
        }) {
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

/// Validate resources up front, then synthesize and queue one structured,
/// tracked presentation timeline.
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
    let resources = match preload_timeline_resources(&timeline.actions, &state, loader) {
        Ok(resources) => resources,
        Err(error) => {
            ctx.mark_failed();
            warn!("Structured presentation resource validation failed: {error}");
            return BatchStatus::Failed;
        }
    };
    let spans = match prepare_timeline_spans(&timeline, &state, &resources) {
        Ok(spans) => spans,
        Err(error) => {
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
    let request = SynthesisRequest::new(&chunk.text, settings)
        .with_normalized_acss(acss.style.clone())
        .with_anchors(requested_timeline_anchors(
            &chunk.capitalization_tones,
            actions,
        ))
        .expect("prepared timeline offsets are valid");
    match ctx.engine.synthesize(&request).and_then(|mut result| {
        result.resolve_anchors(&request, descriptor.capabilities.markers.requested_anchors);
        result.degraded_acss = acss.omitted.clone();
        result.validate(&request)?;
        Ok(result)
    }) {
        Ok(result) => {
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
            ctx.mark_failed();
            warn!("Structured timeline synthesis error: {error}");
            true
        }
    }
}

fn preload_timeline_resources(
    actions: &[PresentationTimelineAction],
    state: &TtsState,
    loader: &AudioFileLoader,
) -> Result<HashMap<String, AudioBuffer>, String> {
    let mut resources = HashMap::new();
    for action in actions {
        let audio = match &action.action {
            PresentationAction::Audio { path, pan, .. } => {
                let mut audio = loader
                    .load(std::path::Path::new(path))
                    .map_err(|error| format!("action {}: {error}", action.id))?;
                build_sound_pipeline(state)
                    .process(&mut audio)
                    .map_err(|error| format!("action {}: {error}", action.id))?;
                apply_action_pan(&mut audio, *pan);
                audio
            }
            PresentationAction::Tone {
                frequency_hz,
                duration_ms,
                pan,
                ..
            } => {
                let mut audio = prepare_tone_audio(*frequency_hz, *duration_ms, state)
                    .map_err(|error| format!("action {}: {error}", action.id))?;
                apply_action_pan(&mut audio, *pan);
                audio
            }
            PresentationAction::Silence { duration_ms } => {
                AudioBuffer::silence(*duration_ms as f32 / 1000.0)
            }
            PresentationAction::SemanticEvent => continue,
        };
        if audio.is_empty() {
            return Err(format!("action {} decoded to empty audio", action.id));
        }
        resources.insert(action.id.clone(), audio);
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
    resources: &HashMap<String, AudioBuffer>,
) -> Result<Vec<PreparedTimelineSpan>, String> {
    timeline
        .spans
        .iter()
        .map(|span| prepare_timeline_span(span, &timeline.actions, state, resources))
        .collect()
}

fn prepare_timeline_span(
    span: &PresentationSpeechSpan,
    actions: &[PresentationTimelineAction],
    state: &TtsState,
    resources: &HashMap<String, AudioBuffer>,
) -> Result<PreparedTimelineSpan, String> {
    let mut acss = span.acss.clone();
    if let Some(rate_offset) = span.rate_offset.filter(|offset| *offset != 0) {
        acss.rate = Some(apply_rate_offset(state.speech_rate, rate_offset));
    }
    let span_actions = actions
        .iter()
        .filter(|action| action.position.span_id() == span.id)
        .collect::<Vec<_>>();
    let source_offsets = span_actions
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
    let mut actions_by_chunk = vec![Vec::new(); chunks.len()];
    for action in span_actions {
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
        let local_offset =
            prepared_offset.clamp(chunk.source_start, chunk.source_end) - chunk.source_start;
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
                audio: resources
                    .get(&action.id)
                    .cloned()
                    .ok_or_else(|| format!("action {} has no prepared resource", action.id))?,
                mode: convert_audio_mode(*mode),
                volume: *volume,
                effect_bus: convert_effect_bus(*effect_bus),
            },
            PresentationAction::Silence { .. } => TimelineChunkActionKind::Audio {
                audio: resources
                    .get(&action.id)
                    .cloned()
                    .ok_or_else(|| format!("action {} has no prepared silence", action.id))?,
                mode: AudioActionMode::Insert,
                volume: 1.0,
                effect_bus: EffectBus::Dry,
            },
            PresentationAction::SemanticEvent => TimelineChunkActionKind::SemanticEvent,
        };
        actions_by_chunk[chunk_index].push(TimelineChunkAction {
            id: action.id.clone(),
            text_offset: local_offset,
            affinity,
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
            .iter()
            .position(|chunk| offset >= chunk.source_start && offset < chunk.source_end)
            .or_else(|| chunks.iter().position(|chunk| chunk.source_start >= offset))
            .unwrap_or(chunks.len() - 1),
        AnchorAffinity::After => chunks
            .iter()
            .rposition(|chunk| offset > chunk.source_start && offset <= chunk.source_end)
            .or_else(|| chunks.iter().rposition(|chunk| chunk.source_end <= offset))
            .unwrap_or(0),
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
    use crate::text::{CAPITAL_TONE_DURATION_MS, CAPITAL_TONE_HZ};
    use omnivox_core::state::CapitalizationPresentation;

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
    fn isolated_capital_uses_selected_presentation() {
        let mut state = TtsState::default();
        for (presentation, expected_chunks, expected_tones) in [
            (CapitalizationPresentation::None, vec![("a", None)], 0),
            (
                CapitalizationPresentation::Spoken,
                vec![("cap", Some("annotate")), ("a", None)],
                0,
            ),
            (CapitalizationPresentation::Tone, vec![("a", None)], 1),
            (
                CapitalizationPresentation::SpokenTone,
                vec![("cap", Some("annotate")), ("a", None)],
                1,
            ),
            (CapitalizationPresentation::Custom, vec![("a", None)], 0),
        ] {
            state.capitalization_presentation = presentation;
            let chunks = prepare_letter_presentation("A", &state);
            assert_eq!(
                chunks
                    .iter()
                    .map(|chunk| (chunk.text.as_str(), chunk.logical_voice_id))
                    .collect::<Vec<_>>(),
                expected_chunks
            );
            let tones = chunks
                .iter()
                .flat_map(|chunk| &chunk.capitalization_tones)
                .collect::<Vec<_>>();
            assert_eq!(tones.len(), expected_tones);
            if let Some(tone) = tones.first() {
                assert_eq!(tone.frequency_hz, CAPITAL_TONE_HZ);
                assert_eq!(tone.duration_ms, CAPITAL_TONE_DURATION_MS);
            }
        }

        state.capitalization_presentation = CapitalizationPresentation::SpokenTone;
        let chunks = prepare_letter_presentation("a", &state);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "a");
        assert_eq!(chunks[0].logical_voice_id, None);
        assert!(chunks[0].capitalization_tones.is_empty());
    }

    #[test]
    fn test_canonicalize_synthesis_result_empty() {
        let tts_buf = omnivox_tts::AudioBuffer::empty();
        let audio_buf = canonicalize_synthesis_result(result(tts_buf)).audio;
        assert!(audio_buf.is_empty());
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
            let mut state = TtsState::default();
            state.tone_volume = volume;
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

            let structured = preload_timeline_resources(
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
            )
            .unwrap();

            let expected_peak = unity_peak * volume;
            for (path, audio) in [
                ("legacy", &legacy),
                ("capital", &capital_resources[0].audio),
                ("structured", structured.get("structured").unwrap()),
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
            reverb: Some(0.4),
            echo: Some(0.6),
        });

        assert!((parameters.gain - 1.0).abs() < 0.000_001);
        assert_eq!(parameters.low_pass_hz, Some(200.0));
        assert!((parameters.high_pass_hz.unwrap() - 3_000.0).abs() < 0.001);
        assert_eq!(parameters.pan, -0.5);
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
            .render_window(&result.audio, &timeline, &resources, true)
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
            .render_window(&result.audio, &timeline, &resources, true)
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
        let actions = vec![
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
        let resources = HashMap::from([("opening-cue".to_owned(), AudioBuffer::silence(0.01))]);

        let prepared =
            prepare_timeline_span(&span, &actions, &TtsState::default(), &resources).unwrap();

        assert_eq!(prepared.chunks.len(), 2);
        assert_eq!(prepared.actions[0][0].id, "opening-cue");
        assert_eq!(prepared.actions[0][0].text_offset, 0);
        assert_eq!(prepared.actions[1][0].id, "sixteenth-word");
        assert_eq!(prepared.actions[1][0].text_offset, 0);
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
