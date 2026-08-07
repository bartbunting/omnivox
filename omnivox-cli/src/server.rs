//! Protocol server: synthesis worker thread, reader loop, command dispatch.

use anyhow::Result;
use omnivox_audio::{
    AudioControl, AudioFileLoader, PlaybackStatus, PlaybackTicket, PostSynthesisProcessor,
    StreamType, TimelineAudioRenderer,
};
use omnivox_core::{
    parse_command, state::{ChannelMode, PunctuationLevel}, Command, CommandId, QueueItem, TtsState,
};
use omnivox_tts::contracts::{
    AcssDimension, EngineDescriptor, FallbackPolicy, LogicalVoiceDefinition, NormalizedAcss,
    PhysicalVoiceId, PostSynthesisDimension, PostSynthesisStyle, VoiceSelector,
};
use omnivox_tts::control::{
    decode_request, format_control_event, process_control_request, ControlRequest,
    ControlErrorCode, ControlResponse, ControlResponseEnvelope, PreviewStatus,
    CONTROL_PROTOCOL_VERSION, MAX_PREVIEW_TEXT_BYTES,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::logical_voices::{LogicalVoiceBinding, LogicalVoiceRegistry};
use omnivox_tts::routing_policy::RoutingPolicyRegistry;
use omnivox_tts::timeline_protocol::PresentationTimelineEnvelope;
use omnivox_tts::{TtsEngine, TtsSettings};
use std::io::{self, BufRead, Write};
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::health::RuntimeEngineHealth;
use crate::marker_events::{MarkerDispatchContext, MarkerEventOutput};
use crate::pipeline::{
    build_sound_pipeline, process_batch, process_presentation_timeline, process_preview,
    synthesize_chunk_with_tones, BatchStatus, SynthCtx,
};
use crate::routing::{legacy_voice_for_engine, LogicalVoiceRoutingSnapshot};
use crate::text::{
    chunk_prepared_speech, normalize_rate, parse_resource_path, prepare_speech_text,
    CapitalizationTone, CAPITAL_TONE_DURATION_MS, CAPITAL_TONE_HZ,
};
use crate::transaction::{
    prefer_newer, prefer_newer_timeline, PreparedPresentation, PreparedStructuredPresentation,
    PresentationGenerations,
};

const PRESENTATION_COALESCE_WINDOW: Duration = Duration::from_millis(2);
const TRACKED_STATUS_PREFIX: &str = "__EMACSVOX_TRACKED__";
const PREVIEW_LOGICAL_VOICE_ID: &str = "omnivox.preview";

// ---------------------------------------------------------------------------
// Synthesis request types
// ---------------------------------------------------------------------------

/// Messages sent from the reader thread to the synthesis worker.
///
/// Each request carries a `gen` (generation) stamp. The worker compares it
/// against the shared `gen_counter` before and after each synthesis call; if
/// the counter has advanced (because the reader processed a `s` / `tts_say`
/// interrupt), the request is abandoned and no audio is queued.
pub enum SynthRequest {
    /// Synthesize and play a batch of queued items (from `q`/`c`/`t`/`sh`/`a` + `d`).
    Batch {
        items: Vec<QueueItem>,
        state: TtsState,
        logical_voice_routing: LogicalVoiceRoutingSnapshot,
        tracking: Option<DispatchTracking>,
        gen: u64,
    },
    /// Render one atomic structured presentation with marker v2 tracking.
    Timeline {
        timeline: PresentationTimelineEnvelope,
        state: TtsState,
        logical_voice_routing: LogicalVoiceRoutingSnapshot,
        gen: u64,
    },
    /// Synthesize one explicitly selected voice without mutating server state.
    Preview {
        request_id: u64,
        text: String,
        requested: VoiceSelector,
        state: TtsState,
        logical_voice_routing: LogicalVoiceRoutingSnapshot,
        gen: u64,
    },
    /// Synthesize and play a single string immediately (`tts_say`).
    Immediate {
        text: String,
        state: TtsState,
        preferred_routing: LogicalVoiceRoutingSnapshot,
        gen: u64,
    },
    /// Synthesize and play a single letter (`l`).
    Letter {
        text: String,
        state: TtsState,
        preferred_routing: LogicalVoiceRoutingSnapshot,
        gen: u64,
    },
    /// Play a sound file immediately on the sound stream (`p`).
    PlaySound { path: std::path::PathBuf, state: TtsState, gen: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchTracking {
    Completion(u64),
    Markers(u64),
}

impl DispatchTracking {
    fn identifier(self) -> u64 {
        match self {
            Self::Completion(identifier) | Self::Markers(identifier) => identifier,
        }
    }
}

pub(crate) struct TrackedPlayback {
    completion: PlaybackCompletion,
    status: BatchStatus,
    tickets: Vec<PlaybackTicket>,
}

pub(crate) enum PlaybackCompletion {
    Tracked(u64),
    Preview {
        request_id: u64,
        requested: VoiceSelector,
        realized: Option<PhysicalVoiceId>,
        degraded_acss: Vec<AcssDimension>,
        degraded_effects: Vec<PostSynthesisDimension>,
        message: Option<String>,
    },
}

pub(crate) fn spawn_tracked_playback_reporter(
    marker_output: MarkerEventOutput,
) -> (mpsc::Sender<TrackedPlayback>, std::thread::JoinHandle<()>) {
    let (sender, receiver) = mpsc::channel::<TrackedPlayback>();
    let handle = std::thread::Builder::new()
        .name("omnivox-playback-tracker".to_owned())
        .spawn(move || tracked_playback_reporter(receiver, marker_output))
        .expect("Failed to spawn tracked playback reporter thread");
    (sender, handle)
}

fn tracked_playback_reporter(
    receiver: mpsc::Receiver<TrackedPlayback>,
    marker_output: MarkerEventOutput,
) {
    for playback in receiver {
        let TrackedPlayback { completion, status, tickets } = playback;
        let status = await_tracked_playback(status, tickets);
        marker_output.flush();
        match completion {
            PlaybackCompletion::Tracked(identifier) => write_tracked_status(identifier, status),
            PlaybackCompletion::Preview {
                request_id,
                requested,
                realized,
                degraded_acss,
                degraded_effects,
                message,
            } => write_preview_status(
                request_id,
                status,
                requested,
                realized,
                degraded_acss,
                degraded_effects,
                message,
            ),
        }
    }
}

fn write_control_response(response: &ControlResponseEnvelope) {
    match format_control_event(response) {
        Ok(event) => {
            let mut stdout = io::stdout().lock();
            if let Err(error) = writeln!(stdout, "{}", event).and_then(|_| stdout.flush()) {
                warn!("Could not write Omnivox control response: {}", error);
            }
        }
        Err(error) => warn!("Could not encode Omnivox control response: {}", error),
    }
}

fn write_preview_status(
    request_id: u64,
    status: BatchStatus,
    requested: VoiceSelector,
    realized: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
    degraded_effects: Vec<PostSynthesisDimension>,
    message: Option<String>,
) {
    write_control_response(&preview_response(
        request_id,
        status,
        requested,
        realized,
        degraded_acss,
        degraded_effects,
        message,
    ));
}

fn preview_response(
    request_id: u64,
    status: BatchStatus,
    requested: VoiceSelector,
    realized: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
    degraded_effects: Vec<PostSynthesisDimension>,
    message: Option<String>,
) -> ControlResponseEnvelope {
    let status = match status {
        BatchStatus::Completed => PreviewStatus::Completed,
        BatchStatus::Cancelled => PreviewStatus::Cancelled,
        BatchStatus::Failed => PreviewStatus::Failed,
    };
    ControlResponseEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        request_id: Some(request_id),
        response: ControlResponse::PreviewCompleted {
            status,
            requested,
            realized,
            degraded_acss,
            degraded_effects,
            message,
        },
    }
}

fn write_tracked_status(identifier: u64, status: BatchStatus) {
    let mut stdout = io::stdout().lock();
    if let Err(error) = writeln!(
        stdout,
        "{} {} {}",
        TRACKED_STATUS_PREFIX,
        identifier,
        tracked_status_name(status)
    )
    .and_then(|_| stdout.flush())
    {
        warn!("Could not write tracked playback status: {}", error);
    }
}

fn await_tracked_playback(mut status: BatchStatus, tickets: Vec<PlaybackTicket>) -> BatchStatus {
    for ticket in tickets {
        if ticket.wait() == PlaybackStatus::Cancelled && status == BatchStatus::Completed {
            status = BatchStatus::Cancelled;
        }
    }
    status
}

fn tracked_status_name(status: BatchStatus) -> &'static str {
    match status {
        BatchStatus::Completed => "completed",
        BatchStatus::Cancelled => "cancelled",
        BatchStatus::Failed => "failed",
    }
}

// ---------------------------------------------------------------------------
// Synthesis worker
// ---------------------------------------------------------------------------

/// Worker thread: receive `SynthRequest`s and synthesize them one at a time.
#[allow(clippy::too_many_arguments)]
pub fn synthesis_worker(
    rx: mpsc::Receiver<SynthRequest>,
    gen_counter: Arc<AtomicU64>,
    engine: Arc<dyn TtsEngine>,
    engine_registry: Arc<EngineRegistry>,
    runtime_health: Arc<RuntimeEngineHealth>,
    control: Arc<AudioControl>,
    loader: AudioFileLoader,
    tracked_playback_tx: mpsc::Sender<TrackedPlayback>,
    marker_output: MarkerEventOutput,
) {
    for request in rx {
        match request {
            SynthRequest::Batch {
                items,
                state,
                mut logical_voice_routing,
                tracking,
                gen,
            } => {
                let runtime_inventory = runtime_health.snapshot(
                    engine_registry.generation(),
                    engine_registry.inventory(),
                );
                logical_voice_routing.replace_inventory(runtime_inventory.engines);
                let batch_engine = logical_voice_routing
                    .preferred_legacy_engine(&engine_registry, &engine);
                let tickets = Mutex::new(Vec::new());
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let failed = AtomicBool::new(false);
                let marker_dispatch = tracking.and_then(|tracking| match tracking {
                    DispatchTracking::Completion(_) => None,
                    DispatchTracking::Markers(identifier) => Some(MarkerDispatchContext::new(
                        identifier,
                        marker_output.clone(),
                    )),
                });
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*batch_engine,
                    control: &control,
                    playback_tickets: tracking.map(|_| &tickets),
                    presentation_clock: Some(&presentation_clock),
                    pending_overlays: Some(&pending_overlays),
                    timeline_renderer: Some(&timeline_renderer),
                    effect_processor: Some(&effect_processor),
                    marker_dispatch: marker_dispatch.as_ref(),
                    batch_failed: Some(&failed),
                };
                let status = process_batch(
                    items,
                    state,
                    &ctx,
                    &loader,
                    &engine_registry,
                    &runtime_health,
                    logical_voice_routing,
                );
                if let Some(tracking) = tracking {
                    let identifier = tracking.identifier();
                    let playback = TrackedPlayback {
                        completion: PlaybackCompletion::Tracked(identifier),
                        status,
                        tickets: tickets.into_inner().unwrap(),
                    };
                    if tracked_playback_tx.send(playback).is_err() {
                        warn!("Tracked playback reporter stopped before dispatch {identifier}");
                    }
                }
            }

            SynthRequest::Timeline {
                timeline,
                state,
                mut logical_voice_routing,
                gen,
            } => {
                let runtime_inventory = runtime_health.snapshot(
                    engine_registry.generation(),
                    engine_registry.inventory(),
                );
                logical_voice_routing.replace_inventory(runtime_inventory.engines);
                let batch_engine = logical_voice_routing
                    .preferred_legacy_engine(&engine_registry, &engine);
                let tickets = Mutex::new(Vec::new());
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let failed = AtomicBool::new(false);
                let marker_dispatch = MarkerDispatchContext::with_timeline_events(
                    timeline.dispatch_id,
                    marker_output.clone(),
                );
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*batch_engine,
                    control: &control,
                    playback_tickets: Some(&tickets),
                    presentation_clock: Some(&presentation_clock),
                    pending_overlays: Some(&pending_overlays),
                    timeline_renderer: Some(&timeline_renderer),
                    effect_processor: Some(&effect_processor),
                    marker_dispatch: Some(&marker_dispatch),
                    batch_failed: Some(&failed),
                };
                let dispatch_id = timeline.dispatch_id;
                let status = process_presentation_timeline(
                    timeline,
                    state,
                    &ctx,
                    &loader,
                    &engine_registry,
                    &runtime_health,
                    logical_voice_routing,
                );
                let playback = TrackedPlayback {
                    completion: PlaybackCompletion::Tracked(dispatch_id),
                    status,
                    tickets: tickets.into_inner().unwrap(),
                };
                if tracked_playback_tx.send(playback).is_err() {
                    warn!("Tracked playback reporter stopped before timeline {dispatch_id}");
                }
            }

            SynthRequest::Preview {
                request_id,
                text,
                requested,
                state,
                mut logical_voice_routing,
                gen,
            } => {
                let runtime_inventory = runtime_health.snapshot(
                    engine_registry.generation(),
                    engine_registry.inventory(),
                );
                logical_voice_routing.replace_inventory(runtime_inventory.engines);
                let tickets = Mutex::new(Vec::new());
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let failed = AtomicBool::new(false);
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*engine,
                    control: &control,
                    playback_tickets: Some(&tickets),
                    presentation_clock: Some(&presentation_clock),
                    pending_overlays: Some(&pending_overlays),
                    timeline_renderer: Some(&timeline_renderer),
                    effect_processor: Some(&effect_processor),
                    marker_dispatch: None,
                    batch_failed: Some(&failed),
                };
                let result = process_preview(
                    &text,
                    state,
                    &ctx,
                    &engine_registry,
                    &runtime_health,
                    logical_voice_routing,
                    PREVIEW_LOGICAL_VOICE_ID,
                );
                let playback = TrackedPlayback {
                    completion: PlaybackCompletion::Preview {
                        request_id,
                        requested,
                        realized: result.realized,
                        degraded_acss: result.degraded_acss,
                        degraded_effects: result.degraded_effects,
                        message: result.message,
                    },
                    status: result.status,
                    tickets: tickets.into_inner().unwrap(),
                };
                if tracked_playback_tx.send(playback).is_err() {
                    warn!("Preview playback reporter stopped before request {request_id}");
                }
            }

            SynthRequest::Immediate {
                text,
                state,
                mut preferred_routing,
                gen,
            } => {
                let runtime_inventory = runtime_health.snapshot(
                    engine_registry.generation(),
                    engine_registry.inventory(),
                );
                preferred_routing.replace_inventory(runtime_inventory.engines);
                let preferred_engine = preferred_routing
                    .preferred_legacy_engine(&engine_registry, &engine);
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*preferred_engine,
                    control: &control,
                    playback_tickets: None,
                    presentation_clock: Some(&presentation_clock),
                    pending_overlays: Some(&pending_overlays),
                    timeline_renderer: Some(&timeline_renderer),
                    effect_processor: Some(&effect_processor),
                    marker_dispatch: None,
                    batch_failed: None,
                };
                if ctx.is_stale() { continue; }
                let settings = TtsSettings {
                    voice: legacy_voice_for_engine(
                        &*preferred_engine,
                        &state.current_voice,
                    ),
                    rate: state.speech_rate,
                    pitch: state.pitch_multiplier,
                    volume: 1.0,
                };
                let prepared = prepare_speech_text(&text, &state);
                let chunks = chunk_prepared_speech(prepared, 15);
                let count = chunks.len();
                for (i, chunk) in chunks.into_iter().enumerate() {
                    if !synthesize_chunk_with_tones(
                        &chunk.text,
                        &chunk.capitalization_tones,
                        &settings,
                        &state,
                        i == count - 1,
                        i == count - 1,
                        &ctx,
                    ) {
                        break;
                    }
                }
                ctx.flush_overlays();
            }

            SynthRequest::Letter {
                text,
                state,
                mut preferred_routing,
                gen,
            } => {
                let runtime_inventory = runtime_health.snapshot(
                    engine_registry.generation(),
                    engine_registry.inventory(),
                );
                preferred_routing.replace_inventory(runtime_inventory.engines);
                let preferred_engine = preferred_routing
                    .preferred_legacy_engine(&engine_registry, &engine);
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*preferred_engine,
                    control: &control,
                    playback_tickets: None,
                    presentation_clock: Some(&presentation_clock),
                    pending_overlays: Some(&pending_overlays),
                    timeline_renderer: Some(&timeline_renderer),
                    effect_processor: Some(&effect_processor),
                    marker_dispatch: None,
                    batch_failed: None,
                };
                if ctx.is_stale() { continue; }

                let mut letter_state = state.clone();
                letter_state.speech_rate = state.character_rate();

                let is_upper = text.chars().next().is_some_and(|c| c.is_uppercase());
                let capitalization_tones = if is_upper && state.allcaps_beep {
                    vec![CapitalizationTone {
                        id: "capitalization-letter".to_string(),
                        text_offset: 0,
                        frequency_hz: CAPITAL_TONE_HZ,
                        duration_ms: CAPITAL_TONE_DURATION_MS,
                    }]
                } else {
                    Vec::new()
                };
                if is_upper && !state.allcaps_beep {
                    letter_state.pitch_multiplier = 1.5;
                }

                let settings = TtsSettings {
                    voice: legacy_voice_for_engine(
                        &*preferred_engine,
                        &letter_state.current_voice,
                    ),
                    rate: letter_state.speech_rate,
                    pitch: letter_state.pitch_multiplier,
                    volume: 1.0,
                };
                let lowered = text.chars().flat_map(char::to_lowercase).collect::<String>();
                synthesize_chunk_with_tones(
                    &lowered,
                    &capitalization_tones,
                    &settings,
                    &letter_state,
                    true,
                    true,
                    &ctx,
                );
                ctx.flush_overlays();
            }

            SynthRequest::PlaySound { path, state, gen } => {
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    engine: &*engine,
                    control: &control,
                    playback_tickets: None,
                    presentation_clock: None,
                    pending_overlays: None,
                    timeline_renderer: None,
                    effect_processor: None,
                    marker_dispatch: None,
                    batch_failed: None,
                };
                if ctx.is_stale() { continue; }
                match loader.load(&path) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(&state);
                        if let Err(e) = pipeline.process(&mut buf) {
                            warn!("Sound pipeline error: {}", e);
                        }
                        if let Err(e) = ctx.control.queue(StreamType::Sound, &buf) {
                            warn!("Sound queue error: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to load sound {}: {}", path.display(), e),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reader loop
// ---------------------------------------------------------------------------

/// Increment the generation counter, stop audio, and optionally stop every TTS engine.
///
/// `stop_engine` should be `true` only for hard stops (`s` command).  For
/// `tts_say` and `letter`, pass `false` — the generation counter already causes
/// the worker to discard stale results, and calling `stop()` cross-thread
/// while AVSpeechSynthesizer is running on its GCD queue corrupts the synthesizer.
pub fn interrupt(
    current_gen: &mut u64,
    gen_counter: &AtomicU64,
    control: &AudioControl,
    engine_registry: &EngineRegistry,
    stop_speech_only: bool,
    stop_engine: bool,
) {
    *current_gen += 1;
    gen_counter.store(*current_gen, Ordering::Release);
    if stop_speech_only {
        control.stop(StreamType::Speech);
    } else {
        control.stop_all();
    }
    if stop_engine {
        engine_registry.stop_all();
    }
}

/// Reader loop: process stdin commands and drive the synthesis worker.
///
/// Does not own `AudioStreams` — the caller keeps it alive so the `OutputStream`
/// drop guard outlives playback.
#[allow(clippy::too_many_arguments)]
pub fn run_server(
    engine: Arc<dyn TtsEngine>,
    engine_registry: Arc<EngineRegistry>,
    runtime_health: Arc<RuntimeEngineHealth>,
    mut state: TtsState,
    tx: mpsc::Sender<SynthRequest>,
    control: Arc<AudioControl>,
    gen_counter: Arc<AtomicU64>,
    worker_handle: std::thread::JoinHandle<()>,
    tracked_playback_handle: std::thread::JoinHandle<()>,
    marker_event_handle: std::thread::JoinHandle<()>,
) -> Result<()> {
    let mut pending: Vec<QueueItem> = Vec::new();
    let mut current_gen: u64 = 0;
    let mut logical_voices = LogicalVoiceRegistry::default();
    let mut presentation_generations = PresentationGenerations::default();
    let preferred_engine_id = engine.descriptor().id;
    let mut routing_policy = RoutingPolicyRegistry::new(preferred_engine_id.clone());

    info!("Ready to accept commands from stdin");

    let (input_tx, input_rx) = mpsc::channel::<io::Result<String>>();
    let input_handle = std::thread::Builder::new()
        .name("omnivox-stdin".to_owned())
        .spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let failed = line.is_err();
                if input_tx.send(line).is_err() || failed {
                    break;
                }
            }
        })
        .expect("Failed to spawn stdin reader thread");
    let mut deferred_command = None;
    let mut input_closed = false;

    while !input_closed {
        let command = match deferred_command
            .take()
            .map_or_else(|| receive_command(&input_rx), |command| Ok(Some(command)))?
        {
            Some(command) => command,
            None => break,
        };

        if command.id == CommandId::EmacsvoxTimeline {
            let Some(mut selected) = prepare_structured_presentation(
                &presentation_generations,
                &command,
            ) else {
                continue;
            };
            loop {
                match receive_command_until(
                    &input_rx,
                    Instant::now() + PRESENTATION_COALESCE_WINDOW,
                )? {
                    TimedCommand::Command(next) if next.id == CommandId::EmacsvoxTimeline => {
                        if let Some(candidate) =
                            prepare_structured_presentation(&presentation_generations, &next)
                        {
                            let superseded_dispatch = if candidate.generation > selected.generation {
                                selected.timeline.dispatch_id
                            } else {
                                candidate.timeline.dispatch_id
                            };
                            selected = prefer_newer_timeline(selected, candidate);
                            write_tracked_status(superseded_dispatch, BatchStatus::Cancelled);
                        }
                    }
                    TimedCommand::Command(next) if next.id == CommandId::Stop => {
                        debug!(
                            "Stop barrier discarded structured Emacsvox presentation {}",
                            selected.generation
                        );
                        presentation_generations.commit(selected.generation);
                        write_tracked_status(
                            selected.timeline.dispatch_id,
                            BatchStatus::Cancelled,
                        );
                        handle_command(
                            next,
                            &mut state,
                            &mut pending,
                            &mut current_gen,
                            &gen_counter,
                            &engine_registry,
                            &runtime_health,
                            &preferred_engine_id,
                            &mut routing_policy,
                            &mut logical_voices,
                            &control,
                            &tx,
                        );
                        break;
                    }
                    TimedCommand::Command(next) => {
                        execute_structured_presentation(
                            selected,
                            &mut presentation_generations,
                            &state,
                            current_gen,
                            &engine_registry,
                            &routing_policy,
                            &logical_voices,
                            &tx,
                        );
                        deferred_command = Some(next);
                        break;
                    }
                    TimedCommand::Timeout => {
                        execute_structured_presentation(
                            selected,
                            &mut presentation_generations,
                            &state,
                            current_gen,
                            &engine_registry,
                            &routing_policy,
                            &logical_voices,
                            &tx,
                        );
                        break;
                    }
                    TimedCommand::Closed => {
                        execute_structured_presentation(
                            selected,
                            &mut presentation_generations,
                            &state,
                            current_gen,
                            &engine_registry,
                            &routing_policy,
                            &logical_voices,
                            &tx,
                        );
                        input_closed = true;
                        break;
                    }
                }
            }
            continue;
        }

        if command.id != CommandId::EmacsvoxTx {
            handle_command(
                command,
                &mut state,
                &mut pending,
                &mut current_gen,
                &gen_counter,
                &engine_registry,
                &runtime_health,
                &preferred_engine_id,
                &mut routing_policy,
                &mut logical_voices,
                &control,
                &tx,
            );
            continue;
        }

        let Some(mut selected) = prepare_presentation(&presentation_generations, &command) else {
            continue;
        };
        loop {
            match receive_command_until(
                &input_rx,
                Instant::now() + PRESENTATION_COALESCE_WINDOW,
            )? {
                TimedCommand::Command(next) if next.id == CommandId::EmacsvoxTx => {
                    if let Some(candidate) =
                        prepare_presentation(&presentation_generations, &next)
                    {
                        selected = prefer_newer(selected, candidate);
                    }
                }
                TimedCommand::Command(next) if next.id == CommandId::Stop => {
                    debug!(
                        "Stop barrier discarded Emacsvox transaction {}",
                        selected.generation
                    );
                    presentation_generations.commit(selected.generation);
                    handle_command(
                        next,
                        &mut state,
                        &mut pending,
                        &mut current_gen,
                        &gen_counter,
                        &engine_registry,
                        &runtime_health,
                        &preferred_engine_id,
                        &mut routing_policy,
                        &mut logical_voices,
                        &control,
                        &tx,
                    );
                    break;
                }
                TimedCommand::Command(next) => {
                    execute_presentation(
                        selected,
                        &mut presentation_generations,
                        &mut state,
                        &mut pending,
                        &mut current_gen,
                        &gen_counter,
                        &engine_registry,
                        &runtime_health,
                        &preferred_engine_id,
                        &mut routing_policy,
                        &mut logical_voices,
                        &control,
                        &tx,
                    );
                    deferred_command = Some(next);
                    break;
                }
                TimedCommand::Timeout => {
                    execute_presentation(
                        selected,
                        &mut presentation_generations,
                        &mut state,
                        &mut pending,
                        &mut current_gen,
                        &gen_counter,
                        &engine_registry,
                        &runtime_health,
                        &preferred_engine_id,
                        &mut routing_policy,
                        &mut logical_voices,
                        &control,
                        &tx,
                    );
                    break;
                }
                TimedCommand::Closed => {
                    execute_presentation(
                        selected,
                        &mut presentation_generations,
                        &mut state,
                        &mut pending,
                        &mut current_gen,
                        &gen_counter,
                        &engine_registry,
                        &runtime_health,
                        &preferred_engine_id,
                        &mut routing_policy,
                        &mut logical_voices,
                        &control,
                        &tx,
                    );
                    input_closed = true;
                    break;
                }
            }
        }
    }
    let _ = input_handle.join();

    info!("Stdin closed; waiting for synthesis worker to finish");
    drop(tx);
    let _ = worker_handle.join();

    info!("Draining audio output");
    control.drain();
    let _ = tracked_playback_handle.join();
    let _ = marker_event_handle.join();

    info!("Shutting down");
    Ok(())
}

enum TimedCommand {
    Command(Command),
    Timeout,
    Closed,
}

fn parse_input_line(line: &str) -> Option<Command> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    debug!("Received: {}", line);
    match parse_command(line) {
        Ok(command) => Some(command),
        Err(error) => {
            error!("Parse error '{}': {}", line, error);
            None
        }
    }
}

fn receive_command(receiver: &mpsc::Receiver<io::Result<String>>) -> Result<Option<Command>> {
    loop {
        match receiver.recv() {
            Ok(Ok(line)) => {
                if let Some(command) = parse_input_line(&line) {
                    return Ok(Some(command));
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Ok(None),
        }
    }
}

fn receive_command_until(
    receiver: &mpsc::Receiver<io::Result<String>>,
    deadline: Instant,
) -> Result<TimedCommand> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(TimedCommand::Timeout);
        };
        match receiver.recv_timeout(remaining) {
            Ok(Ok(line)) => {
                if let Some(command) = parse_input_line(&line) {
                    return Ok(TimedCommand::Command(command));
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(TimedCommand::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(TimedCommand::Closed),
        }
    }
}

fn prepare_presentation(
    generations: &PresentationGenerations,
    command: &Command,
) -> Option<PreparedPresentation> {
    match generations.prepare(command.args.as_deref().unwrap_or("")) {
        Ok(Some(presentation)) => Some(presentation),
        Ok(None) => {
            debug!("Ignored stale Emacsvox presentation transaction");
            None
        }
        Err(error) => {
            warn!("Invalid Emacsvox presentation transaction: {}", error);
            None
        }
    }
}

fn prepare_structured_presentation(
    generations: &PresentationGenerations,
    command: &Command,
) -> Option<PreparedStructuredPresentation> {
    match generations.prepare_timeline(command.args.as_deref().unwrap_or("")) {
        Ok(Some(presentation)) => Some(presentation),
        Ok(None) => {
            debug!("Ignored stale structured Emacsvox presentation");
            None
        }
        Err(error) => {
            warn!("Invalid structured Emacsvox presentation: {error}");
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_structured_presentation(
    presentation: PreparedStructuredPresentation,
    generations: &mut PresentationGenerations,
    state: &TtsState,
    current_gen: u64,
    engine_registry: &EngineRegistry,
    routing_policy: &RoutingPolicyRegistry,
    logical_voices: &LogicalVoiceRegistry,
    tx: &mpsc::Sender<SynthRequest>,
) {
    debug!(
        "Accepted structured Emacsvox presentation {}",
        presentation.generation
    );
    generations.commit(presentation.generation);
    let dispatch_id = presentation.timeline.dispatch_id;
    if tx
        .send(SynthRequest::Timeline {
            timeline: presentation.timeline,
            state: state.clone(),
            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                logical_voices,
                engine_registry,
                routing_policy,
            ),
            gen: current_gen,
        })
        .is_err()
    {
        write_tracked_status(dispatch_id, BatchStatus::Failed);
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_presentation(
    presentation: PreparedPresentation,
    generations: &mut PresentationGenerations,
    state: &mut TtsState,
    pending: &mut Vec<QueueItem>,
    current_gen: &mut u64,
    gen_counter: &Arc<AtomicU64>,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    preferred_engine_id: &str,
    routing_policy: &mut RoutingPolicyRegistry,
    logical_voices: &mut LogicalVoiceRegistry,
    control: &Arc<AudioControl>,
    tx: &mpsc::Sender<SynthRequest>,
) {
    debug!(
        "Accepted Emacsvox presentation transaction {}",
        presentation.generation
    );
    generations.commit(presentation.generation);
    for command in presentation.commands {
        handle_command(
            command,
            state,
            pending,
            current_gen,
            gen_counter,
            engine_registry,
            runtime_health,
            preferred_engine_id,
            routing_policy,
            logical_voices,
            control,
            tx,
        );
    }
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dispatch_preview(
    request_id: u64,
    text: String,
    selector: VoiceSelector,
    language: Option<String>,
    acss: NormalizedAcss,
    effects: PostSynthesisStyle,
    state: &TtsState,
    gen: u64,
    inventory: &[EngineDescriptor],
    engine_registry: &EngineRegistry,
    routing_policy: &RoutingPolicyRegistry,
    tx: &mpsc::Sender<SynthRequest>,
) {
    let reject = |message: String| {
        write_preview_status(
            request_id,
            BatchStatus::Failed,
            selector.clone(),
            None,
            Vec::new(),
            Vec::new(),
            Some(message),
        );
    };
    if text.is_empty() {
        reject("preview text must not be empty".to_owned());
        return;
    }
    if text.len() > MAX_PREVIEW_TEXT_BYTES {
        reject(format!(
            "preview text exceeds the {MAX_PREVIEW_TEXT_BYTES}-byte limit"
        ));
        return;
    }

    let mut preview_registry = LogicalVoiceRegistry::default();
    let registration = preview_registry.register(
        1,
        vec![LogicalVoiceDefinition {
            id: PREVIEW_LOGICAL_VOICE_ID.to_owned(),
            language,
            preferences: vec![selector.clone()],
            acss,
            effects,
        }],
        FallbackPolicy::default(),
        inventory,
    );
    let registration = match registration {
        Ok(registration) => registration,
        Err(error) => {
            reject(error.to_string());
            return;
        }
    };
    match registration.bindings.as_slice() {
        [LogicalVoiceBinding::Resolved { .. }] => {}
        [LogicalVoiceBinding::Unresolved { error }] => {
            reject(error.to_string());
            return;
        }
        _ => {
            reject("preview route did not produce one binding".to_owned());
            return;
        }
    }

    let request = SynthRequest::Preview {
        request_id,
        text,
        requested: selector.clone(),
        state: state.clone(),
        logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_preview(
            &preview_registry,
            engine_registry,
            routing_policy,
        ),
        gen,
    };
    if tx.send(request).is_err() {
        reject("synthesis worker is unavailable".to_owned());
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_command(
    command: Command,
    state: &mut TtsState,
    pending: &mut Vec<QueueItem>,
    current_gen: &mut u64,
    gen_counter: &Arc<AtomicU64>,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    preferred_engine_id: &str,
    routing_policy: &mut RoutingPolicyRegistry,
    logical_voices: &mut LogicalVoiceRegistry,
    control: &Arc<AudioControl>,
    tx: &mpsc::Sender<SynthRequest>,
) {
    match command.id {
        // --- Queue accumulation (no synthesis yet) ---

        CommandId::Queue => {
            if let Some(text) = command.args {
                debug!("Queue speech: {}", text);
                pending.push(QueueItem::Speech(text));
            }
        }

        CommandId::Code => {
            if let Some(codes) = command.args {
                debug!("Queue codes: {}", codes);
                pending.push(QueueItem::Code(codes));
            }
        }

        CommandId::Tone => {
            if let Some(args) = command.args {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Ok(freq), Ok(dur)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        debug!("Queue tone: {}Hz {}ms", freq, dur);
                        pending.push(QueueItem::Tone { frequency: freq, duration: dur });
                    }
                }
            }
        }

        CommandId::Silence => {
            if let Some(dur_str) = command.args {
                if let Ok(dur) = dur_str.parse::<u32>() {
                    debug!("Queue silence: {}ms", dur);
                    pending.push(QueueItem::Silence { duration: dur });
                }
            }
        }

        CommandId::AudioIcon => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Queue audio icon: {}", path.display());
                        pending.push(QueueItem::AudioIcon { path });
                    }
                    Err(error) => warn!("Invalid audio icon path: {}", error),
                }
            }
        }

        // --- Dispatch: send accumulated items to worker ---

        CommandId::Dispatch => {
            if !pending.is_empty() {
                debug!("Dispatch {} items (gen={})", pending.len(), current_gen);
                let items = mem::take(pending);
                let _ = tx.send(SynthRequest::Batch {
                    items,
                    state: state.clone(),
                    logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    tracking: None,
                    gen: *current_gen,
                });
            }
        }

        CommandId::EmacsvoxTrackedDispatch => {
            let identifier = command
                .args
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|identifier| *identifier > 0);
            if let Some(identifier) = identifier {
                debug!(
                    "Tracked dispatch {} with {} items (gen={})",
                    identifier,
                    pending.len(),
                    current_gen
                );
                let request = SynthRequest::Batch {
                    items: mem::take(pending),
                    state: state.clone(),
                    logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    tracking: Some(DispatchTracking::Completion(identifier)),
                    gen: *current_gen,
                };
                if tx.send(request).is_err() {
                    write_tracked_status(identifier, BatchStatus::Failed);
                }
            } else {
                warn!("Invalid tracked dispatch identifier");
            }
        }

        CommandId::EmacsvoxMarkerDispatch => {
            let identifier = command
                .args
                .as_deref()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|identifier| *identifier > 0);
            if let Some(identifier) = identifier {
                debug!(
                    "Marker dispatch {} with {} items (gen={})",
                    identifier,
                    pending.len(),
                    current_gen
                );
                let request = SynthRequest::Batch {
                    items: mem::take(pending),
                    state: state.clone(),
                    logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    tracking: Some(DispatchTracking::Markers(identifier)),
                    gen: *current_gen,
                };
                if tx.send(request).is_err() {
                    write_tracked_status(identifier, BatchStatus::Failed);
                }
            } else {
                warn!("Invalid marker dispatch identifier");
            }
        }

        // --- Interrupting commands ---

        CommandId::Stop => {
            debug!("Stop");
            interrupt(current_gen, gen_counter, control, engine_registry, false, true);
            pending.clear();
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                debug!("tts_say: {}", text);
                interrupt(current_gen, gen_counter, control, engine_registry, true, false);
                let _ = tx.send(SynthRequest::Immediate {
                    text,
                    state: state.clone(),
                    preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    gen: *current_gen,
                });
            }
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Letter: {}", letter);
                interrupt(current_gen, gen_counter, control, engine_registry, true, false);
                let _ = tx.send(SynthRequest::Letter {
                    text: letter,
                    state: state.clone(),
                    preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    gen: *current_gen,
                });
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Play sound: {}", path.display());
                        let _ = tx.send(SynthRequest::PlaySound {
                            path,
                            state: state.clone(),
                            gen: *current_gen,
                        });
                    }
                    Err(error) => warn!("Invalid sound path: {}", error),
                }
            }
        }

        CommandId::Version => {
            let version_text = format!(
                "Omnivox version {}",
                crate::VERSION.replace('.', " dot ")
            );
            let _ = tx.send(SynthRequest::Immediate {
                text: version_text,
                state: state.clone(),
                preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                    logical_voices,
                    engine_registry,
                    routing_policy,
                ),
                gen: *current_gen,
            });
        }

        CommandId::OmnivoxControl => {
            let inventory = runtime_health.snapshot(
                engine_registry.generation(),
                engine_registry.inventory(),
            );
            let payload = command.args.as_deref().unwrap_or("");
            let live_request = decode_request(payload).ok().and_then(|request| {
                if request.protocol_version != CONTROL_PROTOCOL_VERSION {
                    return None;
                }
                Some(request)
            });
            match live_request.map(|request| (request.request_id, request.request)) {
                Some((request_id, ControlRequest::Preview {
                    text,
                    selector,
                    language,
                    acss,
                    effects,
                })) => {
                    let projected =
                        routing_policy.project_inventory(inventory.engines.clone());
                    dispatch_preview(
                        request_id,
                        text,
                        selector,
                        language,
                        acss,
                        effects,
                        state,
                        *current_gen,
                        &projected,
                        engine_registry,
                        routing_policy,
                        tx,
                    );
                }
                Some((request_id, ControlRequest::RequestEngineRecoveryProbe {
                    engine_id,
                })) => {
                    let response = if !inventory.engines.iter().any(|engine| engine.id == engine_id)
                    {
                        ControlResponse::Error {
                            code: ControlErrorCode::InvalidConfiguration,
                            message: format!("unknown engine {engine_id}"),
                        }
                    } else if routing_policy
                        .policy()
                        .disabled_engine_ids
                        .contains(&engine_id)
                    {
                        ControlResponse::Error {
                            code: ControlErrorCode::InvalidConfiguration,
                            message: format!(
                                "engine {engine_id} is disabled by runtime routing policy"
                            ),
                        }
                    } else {
                        match runtime_health.request_probe(&engine_id) {
                            Ok(()) => ControlResponse::EngineRecoveryProbeRequested {
                                inventory_generation: routing_policy.inventory_generation(
                                    runtime_health
                                        .snapshot(
                                            engine_registry.generation(),
                                            engine_registry.inventory(),
                                        )
                                        .generation,
                                ),
                                engine_id,
                            },
                            Err(message) => ControlResponse::Error {
                                code: ControlErrorCode::InvalidConfiguration,
                                message,
                            },
                        }
                    };
                    write_control_response(&ControlResponseEnvelope {
                        protocol_version: CONTROL_PROTOCOL_VERSION,
                        request_id: Some(request_id),
                        response,
                    });
                }
                _ => {
                    let engine_runtime = runtime_health.statuses(
                        &inventory.engines,
                        &routing_policy.policy().disabled_engine_ids,
                    );
                    let response = process_control_request(
                        payload,
                        crate::VERSION,
                        inventory.generation,
                        preferred_engine_id,
                        &inventory.engines,
                        &engine_runtime,
                        logical_voices,
                        routing_policy,
                    );
                    write_control_response(&response);
                }
            }
        }

        CommandId::EmacsvoxTx => {
            warn!("Nested Emacsvox presentation transaction was ignored");
        }

        CommandId::EmacsvoxTimeline => {
            warn!("Structured Emacsvox timeline is not available in legacy command batches");
        }

        // --- State management ---

        CommandId::TtsSetSpeechRate => {
            if let Some(rate) = command.args {
                if let Ok(r) = rate.parse::<f32>() {
                    state.speech_rate = normalize_rate(r);
                    debug!("Speech rate: {}", state.speech_rate);
                }
            }
        }

        CommandId::TtsSetVoice => {
            if let Some(voice) = command.args {
                debug!("Voice: {}", voice);
                state.current_voice = voice;
            }
        }

        CommandId::TtsSetPitchMultiplier => {
            if let Some(pitch) = command.args {
                if let Ok(p) = pitch.parse::<f32>() {
                    state.pitch_multiplier = p;
                    debug!("Pitch: {}", p);
                }
            }
        }

        CommandId::TtsSetVoiceVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.voice_volume = v;
                }
            }
        }

        CommandId::TtsSetToneVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.tone_volume = v;
                }
            }
        }

        CommandId::TtsSetSoundVolume => {
            if let Some(vol) = command.args {
                if let Ok(v) = vol.parse::<f32>() {
                    state.sound_volume = v;
                }
            }
        }

        CommandId::TtsSetCharacterScale => {
            if let Some(scale) = command.args {
                if let Ok(s) = scale.parse::<f32>() {
                    state.character_scale = s;
                }
            }
        }

        CommandId::TtsSplitCaps => {
            if let Some(flag) = command.args {
                state.split_caps = flag == "1";
            }
        }

        CommandId::TtsAllCapsBeep => {
            if let Some(flag) = command.args {
                state.allcaps_beep = flag == "1";
            }
        }

        CommandId::TtsSetPunctuations => {
            if let Some(level) = command.args {
                if let Some(punct) = PunctuationLevel::parse(&level) {
                    state.punctuation_level = punct;
                }
            }
        }

        CommandId::TtsSyncState => {
            if let Some(args) = command.args {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.len() >= 4 {
                    if let Some(punct) = PunctuationLevel::parse(parts[0]) {
                        state.punctuation_level = punct;
                    }
                    state.split_caps = parts[1] == "1";
                    state.allcaps_beep = parts[2] == "1";
                    if let Ok(r) = parts[3].parse::<f32>() {
                        state.speech_rate = normalize_rate(r);
                    }
                }
            }
        }

        CommandId::TtsSetSpeechChannel => {
            if let Some(target) = command.args {
                if let Some(mode) = ChannelMode::parse(&target) {
                    state.speech_routing.channel_mode = mode;
                    debug!("Speech channel: {}", target);
                } else {
                    warn!("Invalid tts_set_speech_channel value: {}", target);
                }
            }
        }

        CommandId::TtsSetNotificationChannel => {
            if let Some(target) = command.args {
                if let Some(mode) = ChannelMode::parse(&target) {
                    state.notification_routing.channel_mode = mode;
                    debug!("Notification channel: {}", target);
                } else {
                    warn!("Invalid tts_set_notification_channel value: {}", target);
                }
            }
        }

        CommandId::TtsReset => {
            debug!("Reset");
            interrupt(current_gen, gen_counter, control, engine_registry, false, true);
            state.reset();
            pending.clear();
        }

        CommandId::TtsExit => {
            info!("Exit command received");
            std::process::exit(0);
        }

        CommandId::SetLang | CommandId::SetNextLang | CommandId::SetPreviousLang | CommandId::SetPreferredLang => {
            debug!("Language switching not yet implemented: {:?} {:?}", command.id, command.args);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tracked_dispatch_preserves_worker_terminal_status() {
        for status in [
            BatchStatus::Completed,
            BatchStatus::Cancelled,
            BatchStatus::Failed,
        ] {
            assert_eq!(
                await_tracked_playback(status, Vec::new()),
                status
            );
        }
    }

    #[test]
    fn tracked_terminal_status_names_match_emacsvox_protocol() {
        assert_eq!(tracked_status_name(BatchStatus::Completed), "completed");
        assert_eq!(tracked_status_name(BatchStatus::Cancelled), "cancelled");
        assert_eq!(tracked_status_name(BatchStatus::Failed), "failed");
    }

    #[test]
    fn preview_response_preserves_route_and_terminal_status() {
        let requested = VoiceSelector::Exact(PhysicalVoiceId::new("winrt", "David"));
        let response = preview_response(
            91,
            BatchStatus::Cancelled,
            requested.clone(),
            Some(PhysicalVoiceId::new("winrt", "David")),
            vec![AcssDimension::Richness],
            Vec::new(),
            None,
        );

        assert!(matches!(
            response,
            ControlResponseEnvelope {
                request_id: Some(91),
                response: ControlResponse::PreviewCompleted {
                    status: PreviewStatus::Cancelled,
                    requested: actual_requested,
                    degraded_acss,
                    ..
                },
                ..
            } if actual_requested == requested
                && degraded_acss == vec![AcssDimension::Richness]
        ));
    }
}
