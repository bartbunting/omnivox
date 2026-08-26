//! Protocol server: synthesis worker thread, reader loop, command dispatch.

use anyhow::Result;
use omnivox_audio::{
    AudioBuffer, AudioControl, AudioFileLoader, PlaybackStatus, PlaybackTicket,
    PostSynthesisProcessor, StreamType, TimelineAudioRenderer, ToneGenerator,
};
use omnivox_core::{
    parse_command, parse_presentation_tone_arguments, parse_tone_arguments,
    state::{CapitalizationPresentation, ChannelMode, PunctuationLevel},
    Command, CommandId, QueueItem, TonePlacement, TtsState,
};
use omnivox_tts::contracts::{
    apply_rate_offset, AcssDimension, EngineDescriptor, FallbackPolicy, LogicalVoiceDefinition,
    NormalizedAcss, PhysicalVoiceId, PostSynthesisDimension, PostSynthesisStyle, VoiceSelector,
    MAX_RATE_OFFSET_POINTS, MIN_RATE_OFFSET_POINTS,
};
use omnivox_tts::control::{
    decode_request, format_control_event, process_control_request, ControlErrorCode,
    ControlRequest, ControlResponse, ControlResponseEnvelope, PreviewStatus,
    CONTROL_PROTOCOL_VERSION, MAX_PREVIEW_TEXT_BYTES,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::logical_voices::{LogicalVoiceBinding, LogicalVoiceRegistry};
use omnivox_tts::routing_policy::RoutingPolicyRegistry;
use omnivox_tts::timeline_protocol::{
    PresentationAction, PresentationDeliveryPolicy, PresentationEffectDirective,
    PresentationTimelineEnvelope,
};
use omnivox_tts::{SynthesisCancellationToken, TtsEngine};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::health::RuntimeEngineHealth;
use crate::marker_events::{MarkerDispatchContext, MarkerEventOutput};
use crate::pipeline::{
    build_sound_pipeline, build_tone_pipeline, process_batch, process_letter,
    process_presentation_timeline, process_preview, validate_presentation_timeline_action_windows,
    BatchStatus, SynthCtx,
};
use crate::routing::LogicalVoiceRoutingSnapshot;
use crate::text::{normalize_rate, parse_resource_path};
use crate::transaction::{
    prefer_newer, select_adjacent_timeline, AdjacentTimelineSelection, MultipartTimelineAssembler,
    PreparedPresentation, PreparedStructuredPresentation, PresentationGenerations,
    StructuredTimelineRejectionKind,
};
use crate::work_queue::{
    bounded_work_queue, BoundedWork, RetiredWork, RetirementReason, WorkQueueLimits,
    WorkQueueReceiver, WorkQueueSender,
};

/// Quiet window for collapsing replaceable interactive timelines.
///
/// This runs on the dedicated protocol reader. A separate maximum window
/// prevents an unbroken key-repeat stream from postponing speech indefinitely.
const PRESENTATION_COALESCE_QUIET_WINDOW: Duration = Duration::from_millis(20);
const PRESENTATION_COALESCE_MAX_WINDOW: Duration = Duration::from_millis(80);
const TIMELINE_MULTIPART_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROTOCOL_LINE_BYTES: usize = 512 * 1024;
const INPUT_QUEUE_CAPACITY: usize = 32;
const MAX_PENDING_ITEMS: usize = 4_096;
const MAX_PENDING_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const TRACKED_PLAYBACK_QUEUE_CAPACITY: usize = 32;
const SYNTHESIS_QUEUE_LIMITS: WorkQueueLimits = WorkQueueLimits {
    max_items: 32,
    max_payload_bytes: 32 * 1024 * 1024,
};
const TRACKED_STATUS_PREFIX: &str = "__EMACSVOX_TRACKED__";
const PREVIEW_LOGICAL_VOICE_ID: &str = "omnivox.preview";
const READY_TUNE_NOTES: &[(f32, u32)] = &[(523.25, 55), (659.25, 55), (783.99, 85)];
const READY_TUNE_GAP_SECONDS: f32 = 0.018;
const READY_TUNE_VOLUME: f32 = 0.35;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReplacementDomain {
    protocol_version: u32,
    replacement_key: Option<String>,
}

impl ReplacementDomain {
    fn from_timeline(timeline: &PresentationTimelineEnvelope) -> Option<Self> {
        (timeline.effective_delivery_policy() == PresentationDeliveryPolicy::Replaceable).then(
            || Self {
                protocol_version: timeline.protocol_version,
                replacement_key: timeline.replacement_key.clone(),
            },
        )
    }
}

#[derive(Clone, Default)]
struct KeyedCancellationRegistry {
    active: Arc<Mutex<HashMap<ReplacementDomain, SynthesisCancellationToken>>>,
}

impl KeyedCancellationRegistry {
    fn prepare(&self, domain: ReplacementDomain) -> KeyedCancellationLease {
        KeyedCancellationLease {
            domain,
            token: SynthesisCancellationToken::new(),
            registry: self.clone(),
            active: false,
        }
    }
}

pub(crate) struct KeyedCancellationLease {
    domain: ReplacementDomain,
    token: SynthesisCancellationToken,
    registry: KeyedCancellationRegistry,
    active: bool,
}

impl KeyedCancellationLease {
    fn token(&self) -> &SynthesisCancellationToken {
        &self.token
    }

    fn activate(&mut self) {
        if self.active {
            return;
        }
        let previous = self
            .registry
            .active
            .lock()
            .unwrap()
            .insert(self.domain.clone(), self.token.clone());
        self.active = true;
        if let Some(previous) = previous {
            previous.cancel();
        }
    }
}

impl Drop for KeyedCancellationLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut active = self.registry.active.lock().unwrap();
        if active
            .get(&self.domain)
            .is_some_and(|current| current.same_token(&self.token))
        {
            active.remove(&self.domain);
        }
    }
}

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
        cancellation: Option<KeyedCancellationLease>,
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
    PlaySound {
        path: std::path::PathBuf,
        state: TtsState,
        gen: u64,
    },
}

impl SynthRequest {
    fn commit_admission(&mut self) {
        if let Self::Timeline {
            cancellation: Some(cancellation),
            ..
        } = self
        {
            cancellation.activate();
        }
    }

    fn diagnostic_kind(&self) -> &'static str {
        match self {
            Self::Batch { .. } => "batch",
            Self::Timeline { .. } => "timeline",
            Self::Preview { .. } => "preview",
            Self::Immediate { .. } => "immediate",
            Self::Letter { .. } => "letter",
            Self::PlaySound { .. } => "sound",
        }
    }

    fn diagnostic_identifier(&self) -> Option<u64> {
        match self {
            Self::Batch {
                tracking: Some(tracking),
                ..
            } => Some(tracking.identifier()),
            Self::Timeline { timeline, .. } => Some(timeline.dispatch_id),
            Self::Preview { request_id, .. } => Some(*request_id),
            _ => None,
        }
    }

    fn generation(&self) -> u64 {
        match self {
            Self::Batch { gen, .. }
            | Self::Timeline { gen, .. }
            | Self::Preview { gen, .. }
            | Self::Immediate { gen, .. }
            | Self::Letter { gen, .. }
            | Self::PlaySound { gen, .. } => *gen,
        }
    }
}

impl BoundedWork for SynthRequest {
    fn queued_payload_bytes(&self) -> usize {
        match self {
            Self::Batch {
                items,
                state,
                logical_voice_routing,
                ..
            } => queue_items_payload_bytes(items)
                .saturating_add(state.current_voice.len())
                .saturating_add(logical_voice_routing.queued_payload_bytes()),
            Self::Timeline {
                timeline,
                state,
                logical_voice_routing,
                ..
            } => timeline_payload_bytes(timeline)
                .saturating_add(state.current_voice.len())
                .saturating_add(logical_voice_routing.queued_payload_bytes()),
            Self::Preview {
                text,
                requested,
                state,
                logical_voice_routing,
                ..
            } => text
                .len()
                .saturating_add(voice_selector_payload_bytes(requested))
                .saturating_add(state.current_voice.len())
                .saturating_add(logical_voice_routing.queued_payload_bytes()),
            Self::Immediate {
                text,
                state,
                preferred_routing,
                ..
            }
            | Self::Letter {
                text,
                state,
                preferred_routing,
                ..
            } => text
                .len()
                .saturating_add(state.current_voice.len())
                .saturating_add(preferred_routing.queued_payload_bytes()),
            Self::PlaySound { path, state, .. } => path
                .as_os_str()
                .to_string_lossy()
                .len()
                .saturating_add(state.current_voice.len()),
        }
    }

    fn generation(&self) -> u64 {
        SynthRequest::generation(self)
    }

    fn is_replaceable(&self) -> bool {
        matches!(
            self,
            Self::Timeline { timeline, .. }
                if timeline.effective_delivery_policy()
                    == PresentationDeliveryPolicy::Replaceable
        )
    }

    fn shares_replacement_domain(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (
                Self::Timeline { timeline, .. },
                Self::Timeline {
                    timeline: other,
                    ..
                }
            ) if timeline.shares_replacement_domain(other)
        )
    }
}

pub(crate) fn synthesis_channel() -> (
    WorkQueueSender<SynthRequest>,
    WorkQueueReceiver<SynthRequest>,
) {
    bounded_work_queue(SYNTHESIS_QUEUE_LIMITS)
}

fn queue_items_payload_bytes(items: &[QueueItem]) -> usize {
    items
        .iter()
        .map(queue_item_payload_bytes)
        .fold(std::mem::size_of_val(items), usize::saturating_add)
}

fn queue_item_payload_bytes(item: &QueueItem) -> usize {
    match item {
        QueueItem::Speech(text) | QueueItem::Code(text) => text.len(),
        QueueItem::AudioIcon { path } => path.as_os_str().to_string_lossy().len(),
        QueueItem::Tone { .. } | QueueItem::Silence { .. } => 0,
    }
}

fn timeline_payload_bytes(timeline: &PresentationTimelineEnvelope) -> usize {
    let spans = timeline
        .spans
        .iter()
        .map(|span| {
            span.text
                .len()
                .saturating_add(span.logical_voice_id.as_ref().map_or(0, String::len))
                .saturating_add(match &span.effects {
                    PresentationEffectDirective::Replace { state_id, .. } => state_id.len(),
                    PresentationEffectDirective::Retain | PresentationEffectDirective::End => 0,
                })
        })
        .fold(
            std::mem::size_of_val(timeline.spans.as_slice()),
            usize::saturating_add,
        );
    let actions = timeline
        .actions
        .iter()
        .map(|action| {
            action.id.len().saturating_add(match &action.action {
                PresentationAction::Audio { path, .. } => path.len(),
                PresentationAction::Tone { .. }
                | PresentationAction::Silence { .. }
                | PresentationAction::SemanticEvent => 0,
            })
        })
        .fold(
            std::mem::size_of_val(timeline.actions.as_slice()),
            usize::saturating_add,
        );
    timeline
        .replacement_key
        .as_ref()
        .map_or(0, String::len)
        .saturating_add(spans)
        .saturating_add(actions)
}

fn voice_selector_payload_bytes(selector: &VoiceSelector) -> usize {
    match selector {
        VoiceSelector::Exact(id) => id.engine_id.len().saturating_add(id.voice_id.len()),
        VoiceSelector::EngineDefault { engine_id } => engine_id.len(),
        VoiceSelector::Properties {
            engine_id,
            language,
            ..
        } => engine_id
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(language.as_ref().map_or(0, String::len)),
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOverflow {
    ItemCount { attempted: usize },
    PayloadBytes { attempted: usize },
}

impl std::fmt::Display for PendingOverflow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ItemCount { attempted } => write!(
                formatter,
                "legacy transaction has {attempted} items; limit is {MAX_PENDING_ITEMS}"
            ),
            Self::PayloadBytes { attempted } => write!(
                formatter,
                "legacy transaction has {attempted} payload bytes; limit is {MAX_PENDING_PAYLOAD_BYTES}"
            ),
        }
    }
}

#[derive(Default)]
struct PendingBatch {
    items: Vec<QueueItem>,
    payload_bytes: usize,
    overflow: Option<PendingOverflow>,
}

impl PendingBatch {
    /// Queue one item or atomically poison the pending transaction. Once a
    /// transaction exceeds a limit, later items are ignored until dispatch,
    /// stop, or reset; a partial transaction is never synthesized.
    fn push(&mut self, item: QueueItem) -> Option<PendingOverflow> {
        if self.overflow.is_some() {
            return None;
        }
        let attempted_items = self.items.len().saturating_add(1);
        let attempted_bytes = self
            .payload_bytes
            .saturating_add(queue_item_payload_bytes(&item));
        let overflow = if attempted_items > MAX_PENDING_ITEMS {
            Some(PendingOverflow::ItemCount {
                attempted: attempted_items,
            })
        } else if attempted_bytes > MAX_PENDING_PAYLOAD_BYTES {
            Some(PendingOverflow::PayloadBytes {
                attempted: attempted_bytes,
            })
        } else {
            None
        };
        if let Some(overflow) = overflow {
            self.items.clear();
            self.payload_bytes = 0;
            self.overflow = Some(overflow);
            return Some(overflow);
        }
        self.items.push(item);
        self.payload_bytes = attempted_bytes;
        None
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty() && self.overflow.is_none()
    }

    fn take(&mut self) -> Result<Vec<QueueItem>, PendingOverflow> {
        if let Some(overflow) = self.overflow.take() {
            self.items.clear();
            self.payload_bytes = 0;
            return Err(overflow);
        }
        self.payload_bytes = 0;
        Ok(mem::take(&mut self.items))
    }

    fn clear(&mut self) {
        self.items.clear();
        self.payload_bytes = 0;
        self.overflow = None;
    }
}

pub(crate) struct TrackedPlayback {
    completion: PlaybackCompletion,
    status: BatchStatus,
    tickets: Vec<PlaybackTicket>,
    // Keep the registry lease alive until every tagged source is terminal.
    cancellation: Option<KeyedCancellationLease>,
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
) -> (
    mpsc::SyncSender<TrackedPlayback>,
    std::thread::JoinHandle<()>,
) {
    let (sender, receiver) = tracked_playback_channel();
    let handle = std::thread::Builder::new()
        .name("omnivox-playback-tracker".to_owned())
        .spawn(move || tracked_playback_reporter(receiver, marker_output))
        .expect("Failed to spawn tracked playback reporter thread");
    (sender, handle)
}

fn tracked_playback_channel() -> (
    mpsc::SyncSender<TrackedPlayback>,
    mpsc::Receiver<TrackedPlayback>,
) {
    mpsc::sync_channel(TRACKED_PLAYBACK_QUEUE_CAPACITY)
}

fn tracked_playback_reporter(
    receiver: mpsc::Receiver<TrackedPlayback>,
    marker_output: MarkerEventOutput,
) {
    for playback in receiver {
        let TrackedPlayback {
            completion,
            status,
            tickets,
            cancellation,
        } = playback;
        let status = await_tracked_playback(status, tickets);
        let record = match completion {
            PlaybackCompletion::Tracked(identifier) => tracked_status_record(identifier, status),
            PlaybackCompletion::Preview {
                request_id,
                requested,
                realized,
                degraded_acss,
                degraded_effects,
                message,
            } => preview_status_record(
                request_id,
                status,
                requested,
                realized,
                degraded_acss,
                degraded_effects,
                message,
            ),
        };
        if !marker_output.emit_terminal(record) {
            warn!("Playback reporter stopped before a terminal record was written");
        }
        drop(cancellation);
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

fn deprecated_command_response(command: &CommandId) -> Option<ControlResponseEnvelope> {
    let (name, replacement) = match command {
        CommandId::SetLang => ("set_lang", "register language-aware logical voices"),
        CommandId::SetNextLang => ("set_next_lang", "select a language-aware logical voice"),
        CommandId::SetPreviousLang => {
            ("set_previous_lang", "select a language-aware logical voice")
        }
        CommandId::SetPreferredLang => (
            "set_preferred_lang",
            "configure logical-voice language selectors",
        ),
        CommandId::TtsSetNotificationChannel => (
            "tts_set_notification_channel",
            "start a separate Omnivox process with OMNIVOX_AUDIO_TARGET",
        ),
        _ => return None,
    };
    Some(ControlResponseEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        request_id: None,
        response: ControlResponse::Error {
            code: ControlErrorCode::UnsupportedOperation,
            message: format!("deprecated command {name} is unsupported; {replacement}"),
        },
    })
}

fn reject_deprecated_command(command: &CommandId) {
    if let Some(response) = deprecated_command_response(command) {
        if let ControlResponse::Error { message, .. } = &response.response {
            warn!("{message}");
        }
        write_control_response(&response);
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
    let record = preview_status_record(
        request_id,
        status,
        requested,
        realized,
        degraded_acss,
        degraded_effects,
        message,
    );
    write_stdout_record(&record, "preview status");
}

#[allow(clippy::too_many_arguments)]
fn preview_status_record(
    request_id: u64,
    status: BatchStatus,
    requested: VoiceSelector,
    realized: Option<PhysicalVoiceId>,
    degraded_acss: Vec<AcssDimension>,
    degraded_effects: Vec<PostSynthesisDimension>,
    message: Option<String>,
) -> String {
    let fallback_requested = requested.clone();
    let response = preview_response(
        request_id,
        status,
        requested,
        realized,
        degraded_acss,
        degraded_effects,
        message,
    );
    match format_control_event(&response) {
        Ok(record) => record,
        Err(error) => {
            warn!("Could not encode preview terminal status: {}", error);
            let fallback = preview_response(
                request_id,
                BatchStatus::Failed,
                fallback_requested,
                None,
                Vec::new(),
                Vec::new(),
                Some("preview terminal metadata exceeded the output limit".to_owned()),
            );
            format_control_event(&fallback)
                .expect("bounded fallback preview terminal status must encode")
        }
    }
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
    let record = tracked_status_record(identifier, status);
    write_stdout_record(&record, "tracked playback status");
}

fn tracked_status_record(identifier: u64, status: BatchStatus) -> String {
    format!(
        "{} {} {}",
        TRACKED_STATUS_PREFIX,
        identifier,
        tracked_status_name(status)
    )
}

fn write_stdout_record(record: &str, kind: &str) {
    let mut stdout = io::stdout().lock();
    if let Err(error) = writeln!(stdout, "{}", record).and_then(|_| stdout.flush()) {
        warn!("Could not write {}: {}", kind, error);
    }
}

fn enqueue_synthesis(tx: &WorkQueueSender<SynthRequest>, request: SynthRequest) -> bool {
    let outcome = if matches!(
        &request,
        SynthRequest::Timeline {
            cancellation: Some(_),
            ..
        }
    ) {
        tx.try_send_with_commit(request, SynthRequest::commit_admission)
    } else {
        tx.try_send(request)
    };
    for retired in outcome.retired {
        report_retired_synthesis(retired);
    }
    outcome.accepted
}

fn cancel_queued_synthesis_before(tx: &WorkQueueSender<SynthRequest>, generation: u64) {
    for retired in tx.retire_before_generation(generation) {
        report_retired_synthesis(retired);
    }
}

fn report_retired_synthesis(retired: RetiredWork<SynthRequest>) {
    let RetiredWork { work, reason } = retired;
    let status = retirement_status(reason);
    let message = retirement_message(reason);
    let request_kind = work.diagnostic_kind();
    let request_identifier = work.diagnostic_identifier();
    match status {
        BatchStatus::Cancelled => info!(
            request_kind,
            request_identifier = ?request_identifier,
            reason = message,
            "Retired queued synthesis request"
        ),
        BatchStatus::Failed => error!(
            request_kind,
            request_identifier = ?request_identifier,
            reason = message,
            "Rejected synthesis request"
        ),
        BatchStatus::Completed => unreachable!("queue retirement cannot complete a request"),
    }

    match work {
        SynthRequest::Batch {
            tracking: Some(tracking),
            ..
        } => write_tracked_status(tracking.identifier(), status),
        SynthRequest::Timeline { timeline, .. } => {
            write_tracked_status(timeline.dispatch_id, status);
        }
        SynthRequest::Preview {
            request_id,
            requested,
            ..
        } => write_preview_status(
            request_id,
            status,
            requested,
            None,
            Vec::new(),
            Vec::new(),
            Some(message.to_owned()),
        ),
        SynthRequest::Batch { tracking: None, .. }
        | SynthRequest::Immediate { .. }
        | SynthRequest::Letter { .. }
        | SynthRequest::PlaySound { .. } => {}
    }
}

fn retirement_status(reason: RetirementReason) -> BatchStatus {
    match reason {
        RetirementReason::Replaced
        | RetirementReason::EvictedForCapacity
        | RetirementReason::StaleGeneration => BatchStatus::Cancelled,
        RetirementReason::Saturated | RetirementReason::ReceiverClosed => BatchStatus::Failed,
    }
}

fn retirement_message(reason: RetirementReason) -> &'static str {
    match reason {
        RetirementReason::Replaced => "superseded by newer replaceable presentation",
        RetirementReason::EvictedForCapacity => {
            "evicted replaceable presentation to preserve bounded queue capacity"
        }
        RetirementReason::StaleGeneration => "cancelled by a newer interrupt generation",
        RetirementReason::Saturated => {
            "synthesis queue is full of ordered or urgent work, or payload limit was exceeded"
        }
        RetirementReason::ReceiverClosed => "synthesis worker is unavailable",
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
    rx: WorkQueueReceiver<SynthRequest>,
    gen_counter: Arc<AtomicU64>,
    engine: Arc<dyn TtsEngine>,
    engine_registry: Arc<EngineRegistry>,
    runtime_health: Arc<RuntimeEngineHealth>,
    control: Arc<AudioControl>,
    loader: AudioFileLoader,
    tracked_playback_tx: mpsc::SyncSender<TrackedPlayback>,
    marker_output: MarkerEventOutput,
) {
    while let Some(request) = rx.recv() {
        let request_kind = request.diagnostic_kind();
        let request_identifier = request.diagnostic_identifier();
        let request_generation = request.generation();
        let request_started_at = Instant::now();
        info!(
            request_kind,
            request_identifier = ?request_identifier,
            generation = request_generation,
            "Synthesis worker accepted request"
        );
        match request {
            SynthRequest::Batch {
                items,
                state,
                mut logical_voice_routing,
                tracking,
                gen,
            } => {
                let runtime_inventory = runtime_health
                    .snapshot(engine_registry.generation(), engine_registry.inventory());
                logical_voice_routing.replace_inventory(runtime_inventory.engines);
                let batch_engine =
                    logical_voice_routing.preferred_legacy_engine(&engine_registry, &engine);
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
                    cancellation: None,
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
                        cancellation: None,
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
                cancellation,
                gen,
            } => {
                let runtime_inventory = runtime_health
                    .snapshot(engine_registry.generation(), engine_registry.inventory());
                logical_voice_routing.replace_inventory(runtime_inventory.engines);
                let batch_engine =
                    logical_voice_routing.preferred_legacy_engine(&engine_registry, &engine);
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
                    cancellation: cancellation.as_ref().map(KeyedCancellationLease::token),
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
                    cancellation,
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
                let runtime_inventory = runtime_health
                    .snapshot(engine_registry.generation(), engine_registry.inventory());
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
                    cancellation: None,
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
                    cancellation: None,
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
                let runtime_inventory = runtime_health
                    .snapshot(engine_registry.generation(), engine_registry.inventory());
                preferred_routing.replace_inventory(runtime_inventory.engines);
                let preferred_engine =
                    preferred_routing.preferred_legacy_engine(&engine_registry, &engine);
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    cancellation: None,
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
                if ctx.is_stale() {
                    continue;
                }
                process_batch(
                    vec![QueueItem::Speech(text)],
                    state,
                    &ctx,
                    &loader,
                    &engine_registry,
                    &runtime_health,
                    preferred_routing,
                );
            }

            SynthRequest::Letter {
                text,
                state,
                mut preferred_routing,
                gen,
            } => {
                let runtime_inventory = runtime_health
                    .snapshot(engine_registry.generation(), engine_registry.inventory());
                preferred_routing.replace_inventory(runtime_inventory.engines);
                let preferred_engine =
                    preferred_routing.preferred_legacy_engine(&engine_registry, &engine);
                let presentation_clock = Mutex::new(Vec::new());
                let pending_overlays = Mutex::new(Vec::new());
                let timeline_renderer = Mutex::new(TimelineAudioRenderer::new());
                let effect_processor = Mutex::new(PostSynthesisProcessor::new());
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    cancellation: None,
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
                if ctx.is_stale() {
                    continue;
                }
                process_letter(
                    &text,
                    state,
                    &ctx,
                    &engine_registry,
                    &runtime_health,
                    preferred_routing,
                );
            }

            SynthRequest::PlaySound { path, state, gen } => {
                let ctx = SynthCtx {
                    gen,
                    gen_counter: &gen_counter,
                    cancellation: None,
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
                if ctx.is_stale() {
                    continue;
                }
                match loader.load(&path) {
                    Ok(mut buf) => {
                        let pipeline = build_sound_pipeline(&state);
                        if let Err(e) = pipeline.process(&mut buf) {
                            warn!("Sound pipeline error: {}", e);
                        }
                        if let Err(e) = ctx
                            .control
                            .queue_if(StreamType::Sound, &buf, || !ctx.is_stale())
                        {
                            warn!("Sound queue error: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to load sound {}: {}", path.display(), e),
                }
            }
        }
        info!(
            request_kind,
            request_identifier = ?request_identifier,
            generation = request_generation,
            elapsed_ms = request_started_at.elapsed().as_millis(),
            "Synthesis worker finished request"
        );
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

fn ready_tune() -> AudioBuffer {
    let mut samples = Vec::new();
    for (index, (frequency_hz, duration_ms)) in READY_TUNE_NOTES.iter().enumerate() {
        let note = ToneGenerator::generate(*frequency_hz, *duration_ms, READY_TUNE_VOLUME);
        samples.extend(note.samples);
        if index + 1 < READY_TUNE_NOTES.len() {
            samples.extend(AudioBuffer::silence(READY_TUNE_GAP_SECONDS).samples);
        }
    }
    AudioBuffer::new(samples)
}

fn play_ready_tune(control: &AudioControl, state: &TtsState) {
    let mut tune = ready_tune();
    if let Err(error) = build_tone_pipeline(state).process(&mut tune) {
        warn!("Could not prepare Omnivox ready tune: {}", error);
        return;
    }
    if let Err(error) = control.queue(StreamType::Tone, &tune) {
        warn!("Could not play Omnivox ready tune: {}", error);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BoundedProtocolLine {
    Line(String),
    Oversized { bytes: usize },
    InvalidUtf8 { bytes: usize },
}

/// Read and drain one newline-delimited record without ever retaining more
/// than one byte beyond the accepted payload limit. The extra byte permits an
/// exact-limit CRLF record while still detecting a true overflow.
fn read_bounded_protocol_line<R: BufRead>(
    reader: &mut R,
) -> io::Result<Option<BoundedProtocolLine>> {
    let mut stored = Vec::new();
    let mut total_content_bytes = 0usize;
    let mut last_content_byte = None;
    let mut saw_input = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if !saw_input {
                return Ok(None);
            }
            break;
        }
        saw_input = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        let content = &available[..content_len];
        total_content_bytes = total_content_bytes.saturating_add(content.len());
        if let Some(last) = content.last() {
            last_content_byte = Some(*last);
        }
        let retained_limit = MAX_PROTOCOL_LINE_BYTES.saturating_add(1);
        let retain = content
            .len()
            .min(retained_limit.saturating_sub(stored.len()));
        stored.extend_from_slice(&content[..retain]);
        let consumed = content_len.saturating_add(usize::from(newline.is_some()));
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    let trailing_carriage_return = last_content_byte == Some(b'\r');
    let logical_bytes = total_content_bytes.saturating_sub(usize::from(trailing_carriage_return));
    if logical_bytes > MAX_PROTOCOL_LINE_BYTES {
        return Ok(Some(BoundedProtocolLine::Oversized {
            bytes: logical_bytes,
        }));
    }
    if trailing_carriage_return {
        stored.pop();
    }
    Ok(Some(match String::from_utf8(stored) {
        Ok(line) => BoundedProtocolLine::Line(line),
        Err(_) => BoundedProtocolLine::InvalidUtf8 {
            bytes: logical_bytes,
        },
    }))
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
    tx: WorkQueueSender<SynthRequest>,
    control: Arc<AudioControl>,
    gen_counter: Arc<AtomicU64>,
    worker_handle: std::thread::JoinHandle<()>,
    tracked_playback_handle: std::thread::JoinHandle<()>,
    marker_event_handle: std::thread::JoinHandle<()>,
) -> Result<()> {
    let mut pending = PendingBatch::default();
    let mut current_gen: u64 = 0;
    let mut logical_voices = LogicalVoiceRegistry::default();
    let mut presentation_generations = PresentationGenerations::default();
    let keyed_cancellations = KeyedCancellationRegistry::default();
    let preferred_engine_id = engine.descriptor().id;
    let mut routing_policy = RoutingPolicyRegistry::new(preferred_engine_id.clone());

    let (input_tx, input_rx) = mpsc::sync_channel::<io::Result<String>>(INPUT_QUEUE_CAPACITY);
    let input_handle = std::thread::Builder::new()
        .name("omnivox-stdin".to_owned())
        .spawn(move || {
            let stdin = io::stdin();
            let mut input = stdin.lock();
            loop {
                match read_bounded_protocol_line(&mut input) {
                    Ok(Some(BoundedProtocolLine::Line(line))) => {
                        if input_tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(BoundedProtocolLine::Oversized { bytes })) => {
                        error!(
                            bytes,
                            limit = MAX_PROTOCOL_LINE_BYTES,
                            "Rejected oversized protocol line"
                        );
                    }
                    Ok(Some(BoundedProtocolLine::InvalidUtf8 { bytes })) => {
                        error!(bytes, "Rejected protocol line containing invalid UTF-8");
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = input_tx.send(Err(error));
                        break;
                    }
                }
            }
        })
        .expect("Failed to spawn stdin reader thread");
    play_ready_tune(&control, &state);
    info!("Ready to accept commands from stdin");
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

        if matches!(
            command.id,
            CommandId::EmacsvoxTimeline | CommandId::EmacsvoxTimelinePart
        ) {
            let mut selected = match read_structured_submission(
                &presentation_generations,
                command,
                &input_rx,
                &state,
            )? {
                StructuredSubmissionRead::Prepared(presentation) => presentation,
                StructuredSubmissionRead::Rejected(rejection) => {
                    report_rejected_structured_submission(&rejection);
                    continue;
                }
                StructuredSubmissionRead::Unowned => continue,
                StructuredSubmissionRead::Aborted(abort) => {
                    retire_aborted_structured_submission(&mut presentation_generations, &abort);
                    deferred_command = abort.deferred_command;
                    input_closed = abort.input_closed;
                    continue;
                }
            };
            let mut selected_cancellation =
                begin_keyed_cancellation(&keyed_cancellations, &selected.timeline);
            let mut burst_started = Instant::now();
            loop {
                if !structured_policy_uses_reader_coalescing(
                    selected.timeline.effective_delivery_policy(),
                ) {
                    execute_structured_presentation(
                        selected,
                        &mut presentation_generations,
                        &state,
                        current_gen,
                        selected_cancellation.take(),
                        &engine_registry,
                        &routing_policy,
                        &logical_voices,
                        &tx,
                    );
                    break;
                }
                match receive_command_until(
                    &input_rx,
                    replaceable_coalescing_deadline(burst_started, Instant::now()),
                )? {
                    TimedCommand::Command(next)
                        if matches!(
                            next.id,
                            CommandId::EmacsvoxTimeline | CommandId::EmacsvoxTimelinePart
                        ) =>
                    {
                        match read_structured_submission(
                            &presentation_generations,
                            next,
                            &input_rx,
                            &state,
                        )? {
                            StructuredSubmissionRead::Prepared(candidate) => {
                                if candidate.generation <= selected.generation {
                                    write_tracked_status(
                                        candidate.timeline.dispatch_id,
                                        BatchStatus::Cancelled,
                                    );
                                } else {
                                    let candidate_cancellation = begin_keyed_cancellation(
                                        &keyed_cancellations,
                                        &candidate.timeline,
                                    );
                                    match select_adjacent_timeline(selected, candidate) {
                                        AdjacentTimelineSelection::Coalesced {
                                            selected: replacement,
                                            cancelled_dispatch_id,
                                        } => {
                                            selected = replacement;
                                            selected_cancellation = candidate_cancellation;
                                            write_tracked_status(
                                                cancelled_dispatch_id,
                                                BatchStatus::Cancelled,
                                            );
                                        }
                                        AdjacentTimelineSelection::PreserveOrder {
                                            current,
                                            candidate,
                                        } => {
                                            execute_structured_presentation(
                                                current,
                                                &mut presentation_generations,
                                                &state,
                                                current_gen,
                                                selected_cancellation.take(),
                                                &engine_registry,
                                                &routing_policy,
                                                &logical_voices,
                                                &tx,
                                            );
                                            selected = candidate;
                                            selected_cancellation = candidate_cancellation;
                                            burst_started = Instant::now();
                                        }
                                    }
                                }
                            }
                            StructuredSubmissionRead::Rejected(rejection) => {
                                report_rejected_structured_submission(&rejection);
                            }
                            StructuredSubmissionRead::Unowned => {}
                            StructuredSubmissionRead::Aborted(abort) => {
                                let stop_barrier = abort
                                    .deferred_command
                                    .as_ref()
                                    .is_some_and(|command| command.id == CommandId::Stop);
                                retire_aborted_structured_submission(
                                    &mut presentation_generations,
                                    &abort,
                                );
                                if stop_barrier {
                                    presentation_generations.commit(selected.generation);
                                    write_tracked_status(
                                        selected.timeline.dispatch_id,
                                        BatchStatus::Cancelled,
                                    );
                                } else {
                                    execute_structured_presentation(
                                        selected,
                                        &mut presentation_generations,
                                        &state,
                                        current_gen,
                                        selected_cancellation.take(),
                                        &engine_registry,
                                        &routing_policy,
                                        &logical_voices,
                                        &tx,
                                    );
                                }
                                deferred_command = abort.deferred_command;
                                input_closed = abort.input_closed;
                                break;
                            }
                        }
                    }
                    TimedCommand::Command(next) if next.id == CommandId::Stop => {
                        debug!(
                            "Stop barrier discarded structured Emacsvox presentation {}",
                            selected.generation
                        );
                        presentation_generations.commit(selected.generation);
                        write_tracked_status(selected.timeline.dispatch_id, BatchStatus::Cancelled);
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
                            selected_cancellation.take(),
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
                            selected_cancellation.take(),
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
                            selected_cancellation.take(),
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
        let burst_started = Instant::now();
        loop {
            match receive_command_until(
                &input_rx,
                replaceable_coalescing_deadline(burst_started, Instant::now()),
            )? {
                TimedCommand::Command(next) if next.id == CommandId::EmacsvoxTx => {
                    if let Some(candidate) = prepare_presentation(&presentation_generations, &next)
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
    match parse_command(line) {
        Ok(command) => {
            debug!(command = ?command.id, bytes = line.len(), "Received protocol command");
            Some(command)
        }
        Err(error) => {
            error!(bytes = line.len(), %error, "Protocol command parse error");
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

fn replaceable_coalescing_deadline(burst_started: Instant, now: Instant) -> Instant {
    (now + PRESENTATION_COALESCE_QUIET_WINDOW).min(burst_started + PRESENTATION_COALESCE_MAX_WINDOW)
}

fn structured_policy_uses_reader_coalescing(policy: PresentationDeliveryPolicy) -> bool {
    policy == PresentationDeliveryPolicy::Replaceable
}

fn begin_keyed_cancellation(
    registry: &KeyedCancellationRegistry,
    timeline: &PresentationTimelineEnvelope,
) -> Option<KeyedCancellationLease> {
    ReplacementDomain::from_timeline(timeline).map(|domain| registry.prepare(domain))
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

enum StructuredSubmissionRead {
    Prepared(PreparedStructuredPresentation),
    Rejected(RejectedStructuredSubmission),
    Unowned,
    Aborted(AbortedStructuredSubmission),
}

struct RejectedStructuredSubmission {
    dispatch_id: u64,
    status: BatchStatus,
}

struct AbortedStructuredSubmission {
    generation: u64,
    dispatch_id: u64,
    status: BatchStatus,
    deferred_command: Option<Command>,
    input_closed: bool,
}

fn read_structured_submission(
    generations: &PresentationGenerations,
    first: Command,
    receiver: &mpsc::Receiver<io::Result<String>>,
    state: &TtsState,
) -> Result<StructuredSubmissionRead> {
    if first.id == CommandId::EmacsvoxTimeline {
        return Ok(validate_structured_action_windows(
            prepare_structured_presentation(generations, &first),
            state,
        ));
    }

    let mut assembler = match MultipartTimelineAssembler::start(first.args.as_deref().unwrap_or(""))
    {
        Ok(assembler) => assembler,
        Err(error) => {
            warn!("Invalid first multipart Emacsvox timeline frame: {error}");
            return Ok(StructuredSubmissionRead::Unowned);
        }
    };
    let generation = assembler.generation();
    let dispatch_id = assembler.dispatch_id();
    let deadline = Instant::now() + TIMELINE_MULTIPART_TIMEOUT;
    loop {
        if assembler.is_complete() {
            let read = match assembler.finish(generations) {
                Ok(Some(presentation)) => StructuredSubmissionRead::Prepared(presentation),
                Ok(None) => StructuredSubmissionRead::Aborted(AbortedStructuredSubmission {
                    generation,
                    dispatch_id,
                    status: BatchStatus::Cancelled,
                    deferred_command: None,
                    input_closed: false,
                }),
                Err(error) => {
                    warn!("Invalid multipart Emacsvox timeline: {error}");
                    StructuredSubmissionRead::Aborted(AbortedStructuredSubmission {
                        generation,
                        dispatch_id,
                        status: BatchStatus::Failed,
                        deferred_command: None,
                        input_closed: false,
                    })
                }
            };
            return Ok(validate_structured_action_windows(read, state));
        }
        match receive_command_until(receiver, deadline)? {
            TimedCommand::Command(next) if next.id == CommandId::EmacsvoxTimelinePart => {
                if let Err(error) = assembler.push(next.args.as_deref().unwrap_or("")) {
                    warn!("Invalid multipart Emacsvox timeline sequence: {error}");
                    let starts_new_submission =
                        MultipartTimelineAssembler::start(next.args.as_deref().unwrap_or(""))
                            .ok()
                            .is_some_and(|next_assembler| {
                                next_assembler.generation() != generation
                                    || next_assembler.dispatch_id() != dispatch_id
                            });
                    return Ok(StructuredSubmissionRead::Aborted(
                        AbortedStructuredSubmission {
                            generation,
                            dispatch_id,
                            status: BatchStatus::Failed,
                            deferred_command: starts_new_submission.then_some(next),
                            input_closed: false,
                        },
                    ));
                }
            }
            TimedCommand::Command(next) => {
                let status = if next.id == CommandId::Stop {
                    BatchStatus::Cancelled
                } else {
                    BatchStatus::Failed
                };
                return Ok(StructuredSubmissionRead::Aborted(
                    AbortedStructuredSubmission {
                        generation,
                        dispatch_id,
                        status,
                        deferred_command: Some(next),
                        input_closed: false,
                    },
                ));
            }
            TimedCommand::Timeout => {
                warn!("Timed out after receiving an incomplete multipart Emacsvox timeline");
                return Ok(StructuredSubmissionRead::Aborted(
                    AbortedStructuredSubmission {
                        generation,
                        dispatch_id,
                        status: BatchStatus::Failed,
                        deferred_command: None,
                        input_closed: false,
                    },
                ));
            }
            TimedCommand::Closed => {
                return Ok(StructuredSubmissionRead::Aborted(
                    AbortedStructuredSubmission {
                        generation,
                        dispatch_id,
                        status: BatchStatus::Failed,
                        deferred_command: None,
                        input_closed: true,
                    },
                ));
            }
        }
    }
}

fn validate_structured_action_windows(
    read: StructuredSubmissionRead,
    state: &TtsState,
) -> StructuredSubmissionRead {
    let StructuredSubmissionRead::Prepared(presentation) = read else {
        return read;
    };
    match validate_presentation_timeline_action_windows(&presentation.timeline, state) {
        Ok(()) => StructuredSubmissionRead::Prepared(presentation),
        Err(error) => {
            warn!(
                "Invalid structured Emacsvox action distribution for dispatch {}: {error}",
                presentation.timeline.dispatch_id
            );
            StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                dispatch_id: presentation.timeline.dispatch_id,
                status: BatchStatus::Failed,
            })
        }
    }
}

fn retire_aborted_structured_submission(
    generations: &mut PresentationGenerations,
    abort: &AbortedStructuredSubmission,
) {
    generations.commit(abort.generation);
    write_tracked_status(abort.dispatch_id, abort.status);
}

fn report_rejected_structured_submission(rejection: &RejectedStructuredSubmission) {
    write_tracked_status(rejection.dispatch_id, rejection.status);
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
) -> StructuredSubmissionRead {
    match generations.prepare_timeline(command.args.as_deref().unwrap_or("")) {
        Ok(presentation) => StructuredSubmissionRead::Prepared(presentation),
        Err(rejection) => {
            match rejection.kind {
                StructuredTimelineRejectionKind::Stale => {
                    debug!(
                        "Rejected stale structured Emacsvox presentation: {}",
                        rejection.message
                    );
                }
                StructuredTimelineRejectionKind::Invalid => {
                    warn!(
                        "Invalid structured Emacsvox presentation: {}",
                        rejection.message
                    );
                }
            }
            rejection
                .identity
                .map_or(StructuredSubmissionRead::Unowned, |identity| {
                    StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                        dispatch_id: identity.dispatch_id(),
                        status: match rejection.kind {
                            StructuredTimelineRejectionKind::Invalid => BatchStatus::Failed,
                            StructuredTimelineRejectionKind::Stale => BatchStatus::Cancelled,
                        },
                    })
                })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_structured_presentation(
    presentation: PreparedStructuredPresentation,
    generations: &mut PresentationGenerations,
    state: &TtsState,
    current_gen: u64,
    cancellation: Option<KeyedCancellationLease>,
    engine_registry: &EngineRegistry,
    routing_policy: &RoutingPolicyRegistry,
    logical_voices: &LogicalVoiceRegistry,
    tx: &WorkQueueSender<SynthRequest>,
) {
    debug!(
        "Accepted structured Emacsvox presentation {}",
        presentation.generation
    );
    generations.commit(presentation.generation);
    enqueue_synthesis(
        tx,
        SynthRequest::Timeline {
            timeline: presentation.timeline,
            state: state.clone(),
            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                logical_voices,
                engine_registry,
                routing_policy,
            ),
            cancellation,
            gen: current_gen,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn execute_presentation(
    presentation: PreparedPresentation,
    generations: &mut PresentationGenerations,
    state: &mut TtsState,
    pending: &mut PendingBatch,
    current_gen: &mut u64,
    gen_counter: &Arc<AtomicU64>,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    preferred_engine_id: &str,
    routing_policy: &mut RoutingPolicyRegistry,
    logical_voices: &mut LogicalVoiceRegistry,
    control: &Arc<AudioControl>,
    tx: &WorkQueueSender<SynthRequest>,
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
    mut acss: NormalizedAcss,
    rate_offset: Option<i16>,
    effects: PostSynthesisStyle,
    state: &TtsState,
    gen: u64,
    inventory: &[EngineDescriptor],
    engine_registry: &EngineRegistry,
    routing_policy: &RoutingPolicyRegistry,
    tx: &WorkQueueSender<SynthRequest>,
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
    if let Err(message) = apply_preview_rate_offset(&mut acss, rate_offset, state.speech_rate) {
        reject(message);
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
    enqueue_synthesis(tx, request);
}

fn apply_preview_rate_offset(
    acss: &mut NormalizedAcss,
    rate_offset: Option<i16>,
    base_rate: f32,
) -> Result<(), String> {
    let Some(rate_offset) = rate_offset else {
        return Ok(());
    };
    if !(MIN_RATE_OFFSET_POINTS..=MAX_RATE_OFFSET_POINTS).contains(&rate_offset) {
        return Err(format!(
            "preview rate offset must be between {MIN_RATE_OFFSET_POINTS} and {MAX_RATE_OFFSET_POINTS} points"
        ));
    }
    if acss.rate.is_some() {
        return Err("preview cannot combine absolute rate and rate offset".to_owned());
    }
    if rate_offset != 0 {
        acss.rate = Some(apply_rate_offset(base_rate, rate_offset));
    }
    Ok(())
}

fn queue_pending_item(pending: &mut PendingBatch, item: QueueItem) {
    if let Some(overflow) = pending.push(item) {
        error!(%overflow, "Rejected oversized legacy transaction");
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_command(
    command: Command,
    state: &mut TtsState,
    pending: &mut PendingBatch,
    current_gen: &mut u64,
    gen_counter: &Arc<AtomicU64>,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    preferred_engine_id: &str,
    routing_policy: &mut RoutingPolicyRegistry,
    logical_voices: &mut LogicalVoiceRegistry,
    control: &Arc<AudioControl>,
    tx: &WorkQueueSender<SynthRequest>,
) {
    match command.id {
        // --- Queue accumulation (no synthesis yet) ---
        CommandId::Queue => {
            if let Some(text) = command.args {
                debug!("Queue speech: {}", text);
                queue_pending_item(pending, QueueItem::Speech(text));
            }
        }

        CommandId::Code => {
            if let Some(codes) = command.args {
                debug!("Queue codes: {}", codes);
                queue_pending_item(pending, QueueItem::Code(codes));
            }
        }

        CommandId::Tone => {
            if let Some(args) = command.args {
                match parse_tone_arguments(&args) {
                    Ok(tone) => {
                        debug!("Queue tone: {}Hz {}ms", tone.frequency_hz, tone.duration_ms);
                        queue_pending_item(
                            pending,
                            QueueItem::Tone {
                                frequency: tone.frequency_hz,
                                duration: tone.duration_ms,
                                placement: TonePlacement::Independent,
                            },
                        );
                    }
                    Err(error) => warn!("Invalid tone: {error}"),
                }
            }
        }

        CommandId::EmacsvoxTone => {
            if let Some(args) = command.args {
                match parse_presentation_tone_arguments(&args) {
                    Ok(tone) => {
                        debug!(
                            "Queue {:?} presentation tone: {}Hz {}ms",
                            tone.placement, tone.frequency_hz, tone.duration_ms
                        );
                        queue_pending_item(
                            pending,
                            QueueItem::Tone {
                                frequency: tone.frequency_hz,
                                duration: tone.duration_ms,
                                placement: tone.placement,
                            },
                        );
                    }
                    Err(error) => warn!("Invalid presentation tone: {error}"),
                }
            }
        }

        CommandId::Silence => {
            if let Some(dur_str) = command.args {
                if let Ok(dur) = dur_str.parse::<u32>() {
                    debug!("Queue silence: {}ms", dur);
                    queue_pending_item(pending, QueueItem::Silence { duration: dur });
                }
            }
        }

        CommandId::AudioIcon => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Queue audio icon: {}", path.display());
                        queue_pending_item(pending, QueueItem::AudioIcon { path });
                    }
                    Err(error) => warn!("Invalid audio icon path: {}", error),
                }
            }
        }

        // --- Dispatch: send accumulated items to worker ---
        CommandId::Dispatch => {
            if !pending.is_empty() {
                debug!("Dispatch {} items (gen={})", pending.len(), current_gen);
                match pending.take() {
                    Ok(items) if !items.is_empty() => {
                        enqueue_synthesis(
                            tx,
                            SynthRequest::Batch {
                                items,
                                state: state.clone(),
                                logical_voice_routing:
                                    LogicalVoiceRoutingSnapshot::capture_with_policy(
                                        logical_voices,
                                        engine_registry,
                                        routing_policy,
                                    ),
                                tracking: None,
                                gen: *current_gen,
                            },
                        );
                    }
                    Ok(_) => {}
                    Err(overflow) => {
                        error!(%overflow, "Discarded oversized legacy transaction at dispatch");
                    }
                }
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
                match pending.take() {
                    Ok(items) => enqueue_synthesis(
                        tx,
                        SynthRequest::Batch {
                            items,
                            state: state.clone(),
                            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                                logical_voices,
                                engine_registry,
                                routing_policy,
                            ),
                            tracking: Some(DispatchTracking::Completion(identifier)),
                            gen: *current_gen,
                        },
                    ),
                    Err(overflow) => {
                        error!(%overflow, identifier, "Rejected oversized tracked transaction");
                        write_tracked_status(identifier, BatchStatus::Failed);
                        false
                    }
                };
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
                match pending.take() {
                    Ok(items) => enqueue_synthesis(
                        tx,
                        SynthRequest::Batch {
                            items,
                            state: state.clone(),
                            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                                logical_voices,
                                engine_registry,
                                routing_policy,
                            ),
                            tracking: Some(DispatchTracking::Markers(identifier)),
                            gen: *current_gen,
                        },
                    ),
                    Err(overflow) => {
                        error!(%overflow, identifier, "Rejected oversized marker transaction");
                        write_tracked_status(identifier, BatchStatus::Failed);
                        false
                    }
                };
            } else {
                warn!("Invalid marker dispatch identifier");
            }
        }

        // --- Interrupting commands ---
        CommandId::Stop => {
            debug!("Stop");
            interrupt(
                current_gen,
                gen_counter,
                control,
                engine_registry,
                false,
                true,
            );
            cancel_queued_synthesis_before(tx, *current_gen);
            pending.clear();
        }

        CommandId::TtsSay => {
            if let Some(text) = command.args {
                debug!("tts_say: {}", text);
                interrupt(
                    current_gen,
                    gen_counter,
                    control,
                    engine_registry,
                    true,
                    false,
                );
                cancel_queued_synthesis_before(tx, *current_gen);
                enqueue_synthesis(
                    tx,
                    SynthRequest::Immediate {
                        text,
                        state: state.clone(),
                        preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                            logical_voices,
                            engine_registry,
                            routing_policy,
                        ),
                        gen: *current_gen,
                    },
                );
            }
        }

        CommandId::Letter => {
            if let Some(letter) = command.args {
                debug!("Letter: {}", letter);
                interrupt(
                    current_gen,
                    gen_counter,
                    control,
                    engine_registry,
                    true,
                    false,
                );
                cancel_queued_synthesis_before(tx, *current_gen);
                enqueue_synthesis(
                    tx,
                    SynthRequest::Letter {
                        text: letter,
                        state: state.clone(),
                        preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                            logical_voices,
                            engine_registry,
                            routing_policy,
                        ),
                        gen: *current_gen,
                    },
                );
            }
        }

        CommandId::PlaySound => {
            if let Some(path) = command.args {
                match parse_resource_path(&path) {
                    Ok(path) => {
                        debug!("Play sound: {}", path.display());
                        enqueue_synthesis(
                            tx,
                            SynthRequest::PlaySound {
                                path,
                                state: state.clone(),
                                gen: *current_gen,
                            },
                        );
                    }
                    Err(error) => warn!("Invalid sound path: {}", error),
                }
            }
        }

        CommandId::Version => {
            let version_text = format!("Omnivox version {}", crate::VERSION.replace('.', " dot "));
            enqueue_synthesis(
                tx,
                SynthRequest::Immediate {
                    text: version_text,
                    state: state.clone(),
                    preferred_routing: LogicalVoiceRoutingSnapshot::capture_with_policy(
                        logical_voices,
                        engine_registry,
                        routing_policy,
                    ),
                    gen: *current_gen,
                },
            );
        }

        CommandId::OmnivoxControl => {
            let inventory =
                runtime_health.snapshot(engine_registry.generation(), engine_registry.inventory());
            let payload = command.args.as_deref().unwrap_or("");
            let live_request = decode_request(payload).ok().and_then(|request| {
                if request.protocol_version != CONTROL_PROTOCOL_VERSION {
                    return None;
                }
                Some(request)
            });
            match live_request.map(|request| (request.request_id, request.request)) {
                Some((
                    request_id,
                    ControlRequest::Preview {
                        text,
                        selector,
                        language,
                        acss,
                        rate_offset,
                        effects,
                    },
                )) => {
                    let projected = routing_policy.project_inventory(inventory.engines.clone());
                    dispatch_preview(
                        request_id,
                        text,
                        selector,
                        language,
                        acss,
                        rate_offset,
                        effects,
                        state,
                        *current_gen,
                        &projected,
                        engine_registry,
                        routing_policy,
                        tx,
                    );
                }
                Some((request_id, ControlRequest::RequestEngineRecoveryProbe { engine_id })) => {
                    let response = if !inventory
                        .engines
                        .iter()
                        .any(|engine| engine.id == engine_id)
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

        CommandId::EmacsvoxTimeline | CommandId::EmacsvoxTimelinePart => {
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

        CommandId::TtsSetCapitalizationPresentation => {
            if let Some(presentation) = command.args {
                if let Some(presentation) = CapitalizationPresentation::parse(&presentation) {
                    state.capitalization_presentation = presentation;
                }
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

        CommandId::TtsReset => {
            debug!("Reset");
            interrupt(
                current_gen,
                gen_counter,
                control,
                engine_registry,
                false,
                true,
            );
            cancel_queued_synthesis_before(tx, *current_gen);
            state.reset();
            pending.clear();
        }

        CommandId::TtsExit => {
            info!("Exit command received");
            std::process::exit(0);
        }

        deprecated @ (CommandId::TtsSetNotificationChannel
        | CommandId::SetLang
        | CommandId::SetNextLang
        | CommandId::SetPreviousLang
        | CommandId::SetPreferredLang) => reject_deprecated_command(&deprecated),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use omnivox_tts::contracts::NormalizedAcss;
    use omnivox_tts::timeline_protocol::{
        encode_multipart_presentation_timeline, encode_presentation_timeline, PresentationAffinity,
        PresentationDeliveryPolicy, PresentationEffectDirective, PresentationLifecycleAnchor,
        PresentationSpeechSpan, PresentationTimelineAction, PresentationTimelineEnvelope,
        PresentationTimelinePosition, MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW,
        PRESENTATION_TIMELINE_PROTOCOL_VERSION,
    };

    use super::*;

    const INVALID_DIRECT_TIMELINE: &str = "eyJwcm90b2NvbF92ZXJzaW9uIjozLCJnZW5lcmF0aW9uIjozMSwiZGlzcGF0Y2hfaWQiOjcyLCJkZWxpdmVyeV9wb2xpY3kiOiJvcmRlcmVkIiwic3BhbnMiOltdLCJhY3Rpb25zIjpbXX0=";

    #[derive(Clone, Default)]
    struct RecordingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl std::io::Write for RecordingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn completed_tracked_playback(identifier: u64) -> TrackedPlayback {
        TrackedPlayback {
            completion: PlaybackCompletion::Tracked(identifier),
            status: BatchStatus::Completed,
            tickets: Vec::new(),
            cancellation: None,
        }
    }

    #[test]
    fn replaceable_coalescing_observes_quiet_and_maximum_windows() {
        let started = Instant::now();

        assert_eq!(
            replaceable_coalescing_deadline(started, started),
            started + PRESENTATION_COALESCE_QUIET_WINDOW
        );
        assert_eq!(
            replaceable_coalescing_deadline(
                started,
                started + PRESENTATION_COALESCE_MAX_WINDOW - Duration::from_millis(5),
            ),
            started + PRESENTATION_COALESCE_MAX_WINDOW
        );
        assert_eq!(
            replaceable_coalescing_deadline(
                started,
                started + PRESENTATION_COALESCE_MAX_WINDOW + Duration::from_millis(5),
            ),
            started + PRESENTATION_COALESCE_MAX_WINDOW
        );
    }

    #[test]
    fn only_replaceable_structured_work_uses_reader_coalescing() {
        assert!(structured_policy_uses_reader_coalescing(
            PresentationDeliveryPolicy::Replaceable
        ));
        assert!(!structured_policy_uses_reader_coalescing(
            PresentationDeliveryPolicy::Ordered
        ));
        assert!(!structured_policy_uses_reader_coalescing(
            PresentationDeliveryPolicy::Urgent
        ));
    }

    #[test]
    fn protocol_reader_accepts_exact_limit_without_a_newline() {
        let input = vec![b'x'; MAX_PROTOCOL_LINE_BYTES];
        let mut reader = Cursor::new(input);

        let line = read_bounded_protocol_line(&mut reader).unwrap();

        assert!(matches!(
            line,
            Some(BoundedProtocolLine::Line(value))
                if value.len() == MAX_PROTOCOL_LINE_BYTES
        ));
        assert_eq!(read_bounded_protocol_line(&mut reader).unwrap(), None);
    }

    #[test]
    fn protocol_reader_counts_multibyte_utf8_bytes() {
        let exact = "é".repeat(MAX_PROTOCOL_LINE_BYTES / 2);
        let mut reader = Cursor::new(exact.as_bytes());

        let line = read_bounded_protocol_line(&mut reader).unwrap();

        assert_eq!(line, Some(BoundedProtocolLine::Line(exact)));
    }

    #[test]
    fn protocol_reader_accepts_exact_limit_with_crlf() {
        let mut input = vec![b'x'; MAX_PROTOCOL_LINE_BYTES];
        input.extend_from_slice(b"\r\n");
        let mut reader = Cursor::new(input);

        let line = read_bounded_protocol_line(&mut reader).unwrap();

        assert!(matches!(
            line,
            Some(BoundedProtocolLine::Line(value))
                if value.len() == MAX_PROTOCOL_LINE_BYTES
        ));
    }

    #[test]
    fn protocol_reader_drains_an_oversized_line_before_the_next_command() {
        let mut input = vec![b'x'; MAX_PROTOCOL_LINE_BYTES + 1];
        input.extend_from_slice(b"\ns\r\n");
        let mut reader = Cursor::new(input);

        assert_eq!(
            read_bounded_protocol_line(&mut reader).unwrap(),
            Some(BoundedProtocolLine::Oversized {
                bytes: MAX_PROTOCOL_LINE_BYTES + 1,
            })
        );
        assert_eq!(
            read_bounded_protocol_line(&mut reader).unwrap(),
            Some(BoundedProtocolLine::Line("s".to_owned()))
        );
    }

    #[test]
    fn protocol_reader_rejects_invalid_utf8_and_resumes_at_the_next_line() {
        let mut reader = Cursor::new([0xff, b'\n', b's', b'\n']);

        let invalid = read_bounded_protocol_line(&mut reader).unwrap();

        assert_eq!(invalid, Some(BoundedProtocolLine::InvalidUtf8 { bytes: 1 }));
        assert_eq!(
            read_bounded_protocol_line(&mut reader).unwrap(),
            Some(BoundedProtocolLine::Line("s".to_owned()))
        );
    }

    #[test]
    fn pending_legacy_transaction_enforces_item_limit_atomically() {
        let mut pending = PendingBatch::default();
        for _ in 0..MAX_PENDING_ITEMS {
            assert_eq!(pending.push(QueueItem::Silence { duration: 1 }), None);
        }

        let overflow = pending.push(QueueItem::Silence { duration: 1 });

        assert_eq!(
            overflow,
            Some(PendingOverflow::ItemCount {
                attempted: MAX_PENDING_ITEMS + 1,
            })
        );
        assert!(matches!(
            pending.take(),
            Err(PendingOverflow::ItemCount { .. })
        ));
        assert!(pending.is_empty());
    }

    #[test]
    fn pending_legacy_transaction_enforces_payload_limit_atomically() {
        let mut pending = PendingBatch::default();
        assert_eq!(
            pending.push(QueueItem::Speech("x".repeat(MAX_PENDING_PAYLOAD_BYTES))),
            None
        );

        let overflow = pending.push(QueueItem::Speech("x".to_owned()));

        assert_eq!(
            overflow,
            Some(PendingOverflow::PayloadBytes {
                attempted: MAX_PENDING_PAYLOAD_BYTES + 1,
            })
        );
        assert!(matches!(
            pending.take(),
            Err(PendingOverflow::PayloadBytes { .. })
        ));
    }

    #[test]
    fn synthesis_queue_limits_match_the_negotiated_operational_policy() {
        assert_eq!(SYNTHESIS_QUEUE_LIMITS.max_items, 32);
        assert_eq!(SYNTHESIS_QUEUE_LIMITS.max_payload_bytes, 32 * 1024 * 1024);
        assert_eq!(INPUT_QUEUE_CAPACITY, 32);
    }

    #[test]
    fn queue_retirement_maps_replacement_to_cancel_and_saturation_to_failure() {
        for reason in [
            RetirementReason::Replaced,
            RetirementReason::EvictedForCapacity,
            RetirementReason::StaleGeneration,
        ] {
            assert_eq!(retirement_status(reason), BatchStatus::Cancelled);
        }
        for reason in [
            RetirementReason::Saturated,
            RetirementReason::ReceiverClosed,
        ] {
            assert_eq!(retirement_status(reason), BatchStatus::Failed);
        }
    }

    fn multipart_commands(generation: u64, dispatch_id: u64) -> Vec<Command> {
        let timeline = PresentationTimelineEnvelope {
            protocol_version: PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation,
            dispatch_id,
            delivery_policy: Some(PresentationDeliveryPolicy::Ordered),
            replacement_key: None,
            spans: vec![PresentationSpeechSpan {
                id: 1,
                text: "café 日本".to_owned(),
                logical_voice_id: None,
                acss: NormalizedAcss::default(),
                rate_offset: None,
                effects: PresentationEffectDirective::Retain,
            }],
            actions: Vec::new(),
        };
        let encoded = encode_multipart_presentation_timeline(&timeline).unwrap();
        let padding = encoded
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'=')
            .count();
        let decoded_bytes = (encoded.len() / 4) * 3 - padding;
        let split = ((encoded.len() / 2) / 4) * 4;
        [&encoded[..split], &encoded[split..]]
            .into_iter()
            .enumerate()
            .map(|(index, fragment)| {
                Command::new(
                    CommandId::EmacsvoxTimelinePart,
                    Some(format!(
                        "{PRESENTATION_TIMELINE_PROTOCOL_VERSION} {generation} {dispatch_id} {index} 2 {decoded_bytes} {fragment}"
                    )),
                )
            })
            .collect()
    }

    fn timeline_envelope(
        generation: u64,
        dispatch_id: u64,
        policy: PresentationDeliveryPolicy,
        replacement_key: Option<&str>,
    ) -> PresentationTimelineEnvelope {
        PresentationTimelineEnvelope {
            protocol_version: PRESENTATION_TIMELINE_PROTOCOL_VERSION,
            generation,
            dispatch_id,
            delivery_policy: Some(policy),
            replacement_key: replacement_key.map(str::to_owned),
            spans: vec![PresentationSpeechSpan {
                id: 1,
                text: "queued text".to_owned(),
                logical_voice_id: None,
                acss: NormalizedAcss::default(),
                rate_offset: None,
                effects: PresentationEffectDirective::Retain,
            }],
            actions: Vec::new(),
        }
    }

    fn timeline_request(
        generation: u64,
        dispatch_id: u64,
        policy: PresentationDeliveryPolicy,
        replacement_key: Option<&str>,
    ) -> SynthRequest {
        let engines = EngineRegistry::new();
        SynthRequest::Timeline {
            timeline: timeline_envelope(generation, dispatch_id, policy, replacement_key),
            state: TtsState::default(),
            logical_voice_routing: LogicalVoiceRoutingSnapshot::capture(
                &LogicalVoiceRegistry::default(),
                &engines,
            ),
            cancellation: None,
            gen: generation,
        }
    }

    #[test]
    fn keyed_cancellation_advances_on_activation_and_is_domain_scoped() {
        let registry = KeyedCancellationRegistry::default();
        let mut first = begin_keyed_cancellation(
            &registry,
            &timeline_envelope(
                1,
                11,
                PresentationDeliveryPolicy::Replaceable,
                Some("navigation"),
            ),
        )
        .unwrap();
        assert!(registry.active.lock().unwrap().is_empty());
        first.activate();
        let mut other = begin_keyed_cancellation(
            &registry,
            &timeline_envelope(
                2,
                12,
                PresentationDeliveryPolicy::Replaceable,
                Some("review"),
            ),
        )
        .unwrap();
        other.activate();

        let mut replacement = begin_keyed_cancellation(
            &registry,
            &timeline_envelope(
                3,
                13,
                PresentationDeliveryPolicy::Replaceable,
                Some("navigation"),
            ),
        )
        .unwrap();

        assert!(!first.token().is_cancelled());
        replacement.activate();
        assert!(first.token().is_cancelled());
        assert!(!other.token().is_cancelled());
        assert!(!replacement.token().is_cancelled());
        assert!(begin_keyed_cancellation(
            &registry,
            &timeline_envelope(4, 14, PresentationDeliveryPolicy::Ordered, None),
        )
        .is_none());

        drop(first);
        assert_eq!(registry.active.lock().unwrap().len(), 2);
        drop(replacement);
        assert_eq!(registry.active.lock().unwrap().len(), 1);
        drop(other);
        assert!(registry.active.lock().unwrap().is_empty());
    }

    #[test]
    fn tracked_playback_retains_keyed_cancellation_across_worker_handoff() {
        let registry = KeyedCancellationRegistry::default();
        let mut first = begin_keyed_cancellation(
            &registry,
            &timeline_envelope(
                1,
                11,
                PresentationDeliveryPolicy::Replaceable,
                Some("navigation"),
            ),
        )
        .unwrap();
        first.activate();
        let playback = TrackedPlayback {
            completion: PlaybackCompletion::Tracked(11),
            status: BatchStatus::Completed,
            tickets: Vec::new(),
            cancellation: Some(first),
        };

        let mut replacement = begin_keyed_cancellation(
            &registry,
            &timeline_envelope(
                2,
                12,
                PresentationDeliveryPolicy::Replaceable,
                Some("navigation"),
            ),
        )
        .unwrap();

        assert!(!playback
            .cancellation
            .as_ref()
            .unwrap()
            .token()
            .is_cancelled());
        replacement.activate();
        assert!(playback
            .cancellation
            .as_ref()
            .unwrap()
            .token()
            .is_cancelled());
        assert!(!replacement.token().is_cancelled());
        drop(playback);
        assert_eq!(registry.active.lock().unwrap().len(), 1);
        drop(replacement);
        assert!(registry.active.lock().unwrap().is_empty());
    }

    #[test]
    fn tracked_playback_handoff_is_bounded() {
        let (sender, receiver) = tracked_playback_channel();
        for identifier in 0..TRACKED_PLAYBACK_QUEUE_CAPACITY as u64 {
            assert!(sender
                .try_send(completed_tracked_playback(identifier))
                .is_ok());
        }

        assert!(matches!(
            sender.try_send(completed_tracked_playback(10_000)),
            Err(mpsc::TrySendError::Full(_))
        ));
        drop(receiver);
    }

    #[test]
    fn tracked_reporter_writes_one_terminal_after_earlier_markers() {
        let writer = RecordingWriter::default();
        let written = writer.bytes.clone();
        let (marker_output, marker_handle) =
            crate::marker_events::spawn_marker_event_reporter_with_writer(writer);
        marker_output.emit_test_record("marker").unwrap();
        let (sender, tracker_handle) = spawn_tracked_playback_reporter(marker_output.clone());

        sender.send(completed_tracked_playback(73)).unwrap();
        drop(sender);
        tracker_handle.join().unwrap();
        drop(marker_output);
        marker_handle.join().unwrap();

        assert_eq!(
            String::from_utf8(written.lock().unwrap().clone()).unwrap(),
            "marker\n__EMACSVOX_TRACKED__ 73 completed\n"
        );
    }

    #[test]
    fn rejected_keyed_replacement_keeps_the_active_request_alive() {
        let registry = KeyedCancellationRegistry::default();
        let (sender, receiver) = bounded_work_queue(WorkQueueLimits {
            max_items: 2,
            max_payload_bytes: usize::MAX,
        });
        let active_timeline = timeline_envelope(
            1,
            11,
            PresentationDeliveryPolicy::Replaceable,
            Some("navigation"),
        );
        let active_cancellation = begin_keyed_cancellation(&registry, &active_timeline).unwrap();
        let active_token = active_cancellation.token().clone();
        let engines = EngineRegistry::new();
        assert!(enqueue_synthesis(
            &sender,
            SynthRequest::Timeline {
                timeline: active_timeline,
                state: TtsState::default(),
                logical_voice_routing: LogicalVoiceRoutingSnapshot::capture(
                    &LogicalVoiceRegistry::default(),
                    &engines,
                ),
                cancellation: Some(active_cancellation),
                gen: 1,
            },
        ));
        let active_request = receiver.recv().unwrap();
        assert!(
            sender
                .try_send(timeline_request(
                    2,
                    12,
                    PresentationDeliveryPolicy::Ordered,
                    None,
                ))
                .accepted
        );
        assert!(
            sender
                .try_send(timeline_request(
                    3,
                    13,
                    PresentationDeliveryPolicy::Urgent,
                    None,
                ))
                .accepted
        );

        let replacement_timeline = timeline_envelope(
            4,
            14,
            PresentationDeliveryPolicy::Replaceable,
            Some("navigation"),
        );
        let replacement_cancellation =
            begin_keyed_cancellation(&registry, &replacement_timeline).unwrap();
        let replacement_token = replacement_cancellation.token().clone();
        let rejected = sender.try_send_with_commit(
            SynthRequest::Timeline {
                timeline: replacement_timeline,
                state: TtsState::default(),
                logical_voice_routing: LogicalVoiceRoutingSnapshot::capture(
                    &LogicalVoiceRegistry::default(),
                    &engines,
                ),
                cancellation: Some(replacement_cancellation),
                gen: 4,
            },
            SynthRequest::commit_admission,
        );

        assert!(!rejected.accepted);
        assert_eq!(rejected.retired.len(), 1);
        assert_eq!(rejected.retired[0].reason, RetirementReason::Saturated);
        assert!(!active_token.is_cancelled());
        assert!(!replacement_token.is_cancelled());
        assert_eq!(registry.active.lock().unwrap().len(), 1);
        drop(active_request);
        assert!(registry.active.lock().unwrap().is_empty());
    }

    #[test]
    fn synthesis_queue_coalesces_only_matching_replaceable_timelines() {
        let first = timeline_request(
            1,
            11,
            PresentationDeliveryPolicy::Replaceable,
            Some("navigation"),
        );
        let same_domain = timeline_request(
            2,
            12,
            PresentationDeliveryPolicy::Replaceable,
            Some("navigation"),
        );
        let ordered = timeline_request(3, 13, PresentationDeliveryPolicy::Ordered, None);

        assert!(BoundedWork::is_replaceable(&first));
        assert!(BoundedWork::shares_replacement_domain(&same_domain, &first));
        assert!(!BoundedWork::is_replaceable(&ordered));
        assert!(!BoundedWork::shares_replacement_domain(&ordered, &first));
        assert!(BoundedWork::queued_payload_bytes(&first) >= "queued text".len());
    }

    #[test]
    fn keyed_replacement_preserves_protected_and_other_domain_work() {
        let (sender, receiver) = synthesis_channel();
        for request in [
            timeline_request(1, 11, PresentationDeliveryPolicy::Ordered, None),
            timeline_request(2, 12, PresentationDeliveryPolicy::Urgent, None),
            timeline_request(
                3,
                13,
                PresentationDeliveryPolicy::Replaceable,
                Some("navigation"),
            ),
            timeline_request(
                4,
                14,
                PresentationDeliveryPolicy::Replaceable,
                Some("review"),
            ),
        ] {
            assert!(sender.try_send(request).accepted);
        }

        let outcome = sender.try_send(timeline_request(
            5,
            15,
            PresentationDeliveryPolicy::Replaceable,
            Some("navigation"),
        ));

        assert!(outcome.accepted);
        assert_eq!(outcome.retired.len(), 1);
        assert_eq!(outcome.retired[0].reason, RetirementReason::Replaced);
        assert_eq!(outcome.retired[0].work.diagnostic_identifier(), Some(13));
        assert_eq!(
            (0..4)
                .map(|_| receiver.recv().unwrap().diagnostic_identifier().unwrap())
                .collect::<Vec<_>>(),
            vec![11, 12, 14, 15]
        );
    }

    #[test]
    fn empty_tracked_dispatch_preserves_worker_terminal_status() {
        for status in [
            BatchStatus::Completed,
            BatchStatus::Cancelled,
            BatchStatus::Failed,
        ] {
            assert_eq!(await_tracked_playback(status, Vec::new()), status);
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

    #[test]
    fn deprecated_commands_return_actionable_unsupported_errors() {
        assert_eq!(
            omnivox_core::DEPRECATED_PROTOCOL_COMMANDS,
            omnivox_tts::control::DEPRECATED_PROTOCOL_COMMANDS
        );
        let language = deprecated_command_response(&CommandId::SetLang).unwrap();
        assert!(matches!(
            language.response,
            ControlResponse::Error {
                code: ControlErrorCode::UnsupportedOperation,
                ref message,
            } if message.contains("set_lang") && message.contains("logical voices")
        ));

        let notification =
            deprecated_command_response(&CommandId::TtsSetNotificationChannel).unwrap();
        assert!(matches!(
            notification.response,
            ControlResponse::Error {
                code: ControlErrorCode::UnsupportedOperation,
                ref message,
            } if message.contains("tts_set_notification_channel")
                && message.contains("OMNIVOX_AUDIO_TARGET")
        ));
        assert!(deprecated_command_response(&CommandId::TtsSetSpeechChannel).is_none());
    }

    #[test]
    fn preview_rate_offset_uses_current_rate_without_mutating_it() {
        let mut acss = NormalizedAcss::default();

        apply_preview_rate_offset(&mut acss, Some(-1), 0.75).unwrap();
        assert!((acss.rate.unwrap() - 0.74).abs() < f32::EPSILON);

        let mut faster = NormalizedAcss::default();
        apply_preview_rate_offset(&mut faster, Some(4), 0.75).unwrap();
        assert!((faster.rate.unwrap() - 0.79).abs() < f32::EPSILON);

        let mut neutral = NormalizedAcss::default();
        apply_preview_rate_offset(&mut neutral, Some(0), 0.75).unwrap();
        assert_eq!(neutral.rate, None);
    }

    #[test]
    fn preview_rate_offset_rejects_invalid_or_ambiguous_values() {
        let mut out_of_range = NormalizedAcss::default();
        assert!(apply_preview_rate_offset(&mut out_of_range, Some(21), 0.75).is_err());

        let mut ambiguous = NormalizedAcss {
            rate: Some(0.5),
            ..NormalizedAcss::default()
        };
        assert!(apply_preview_rate_offset(&mut ambiguous, Some(1), 0.75).is_err());
    }

    #[test]
    fn ready_tune_is_short_and_audible() {
        let tune = ready_tune();

        assert!((0.22..0.25).contains(&tune.duration_secs()));
        assert!(tune.samples.iter().any(|sample| sample.abs() > 0.1));
    }

    #[test]
    fn reader_reassembles_multipart_before_returning_one_presentation() {
        let mut commands = multipart_commands(12, 34).into_iter();
        let first = commands.next().unwrap();
        let second = commands.next().unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(format!(
                "emacsvox_timeline_part {}",
                second.args.unwrap()
            )))
            .unwrap();

        let read = read_structured_submission(
            &PresentationGenerations::default(),
            first,
            &receiver,
            &TtsState::default(),
        )
        .unwrap();

        assert!(matches!(
            read,
            StructuredSubmissionRead::Prepared(presentation)
                if presentation.generation == 12
                    && presentation.timeline.dispatch_id == 34
                    && presentation.timeline.spans[0].text == "café 日本"
        ));
    }

    #[test]
    fn reader_rejects_an_action_heavy_window_before_admission() {
        let generations = PresentationGenerations::default();
        let mut timeline = timeline_envelope(18, 40, PresentationDeliveryPolicy::Ordered, None);
        timeline.actions = (0..=MAX_TIMELINE_ACTIONS_PER_SPEECH_WINDOW)
            .map(|index| PresentationTimelineAction {
                id: format!("semantic.{index}"),
                position: PresentationTimelinePosition::SpanBoundary {
                    span_id: 1,
                    affinity: PresentationAffinity::After,
                },
                lifecycle_anchor: PresentationLifecycleAnchor::Run,
                action: PresentationAction::SemanticEvent,
            })
            .collect();
        let command = Command::new(
            CommandId::EmacsvoxTimeline,
            Some(encode_presentation_timeline(&timeline).unwrap()),
        );
        let (_sender, receiver) = mpsc::channel();

        let read =
            read_structured_submission(&generations, command, &receiver, &TtsState::default())
                .unwrap();

        assert!(matches!(
            read,
            StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                dispatch_id: 40,
                status: BatchStatus::Failed,
            })
        ));
    }

    #[test]
    fn direct_rejections_map_owned_dispatches_to_terminal_statuses() {
        let mut generations = PresentationGenerations::default();
        let invalid = Command::new(
            CommandId::EmacsvoxTimeline,
            Some(INVALID_DIRECT_TIMELINE.to_owned()),
        );
        assert!(matches!(
            prepare_structured_presentation(&generations, &invalid),
            StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                dispatch_id: 72,
                status: BatchStatus::Failed,
            })
        ));

        generations.commit(40);
        let stale = Command::new(
            CommandId::EmacsvoxTimeline,
            Some(
                encode_presentation_timeline(&timeline_envelope(
                    40,
                    73,
                    PresentationDeliveryPolicy::Ordered,
                    None,
                ))
                .unwrap(),
            ),
        );
        assert!(matches!(
            prepare_structured_presentation(&generations, &stale),
            StructuredSubmissionRead::Rejected(RejectedStructuredSubmission {
                dispatch_id: 73,
                status: BatchStatus::Cancelled,
            })
        ));

        let unowned = Command::new(CommandId::EmacsvoxTimeline, Some("not-base64".to_owned()));
        assert!(matches!(
            prepare_structured_presentation(&generations, &unowned),
            StructuredSubmissionRead::Unowned
        ));
    }

    #[test]
    fn stop_between_multipart_frames_cancels_the_logical_dispatch() {
        let first = multipart_commands(13, 35).into_iter().next().unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok("s".to_owned())).unwrap();

        let read = read_structured_submission(
            &PresentationGenerations::default(),
            first,
            &receiver,
            &TtsState::default(),
        )
        .unwrap();

        assert!(matches!(
            read,
            StructuredSubmissionRead::Aborted(AbortedStructuredSubmission {
                generation: 13,
                dispatch_id: 35,
                status: BatchStatus::Cancelled,
                deferred_command: Some(Command {
                    id: CommandId::Stop,
                    ..
                }),
                input_closed: false,
            })
        ));
    }

    #[test]
    fn malformed_multipart_part_is_consumed_with_the_failed_submission() {
        let first = multipart_commands(14, 36).into_iter().next().unwrap();
        let duplicate = first.clone();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(format!(
                "emacsvox_timeline_part {}",
                duplicate.args.unwrap()
            )))
            .unwrap();

        let read = read_structured_submission(
            &PresentationGenerations::default(),
            first,
            &receiver,
            &TtsState::default(),
        )
        .unwrap();

        assert!(matches!(
            read,
            StructuredSubmissionRead::Aborted(AbortedStructuredSubmission {
                generation: 14,
                dispatch_id: 36,
                status: BatchStatus::Failed,
                deferred_command: None,
                input_closed: false,
            })
        ));
    }

    #[test]
    fn new_part_zero_after_incomplete_submission_is_preserved() {
        let old_first = multipart_commands(15, 37).into_iter().next().unwrap();
        let mut new_commands = multipart_commands(16, 38).into_iter();
        let new_first = new_commands.next().unwrap();
        let new_second = new_commands.next().unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(format!(
                "emacsvox_timeline_part {}",
                new_first.args.clone().unwrap()
            )))
            .unwrap();
        sender
            .send(Ok(format!(
                "emacsvox_timeline_part {}",
                new_second.args.unwrap()
            )))
            .unwrap();

        let aborted = read_structured_submission(
            &PresentationGenerations::default(),
            old_first,
            &receiver,
            &TtsState::default(),
        )
        .unwrap();
        let StructuredSubmissionRead::Aborted(abort) = aborted else {
            panic!("old incomplete submission was not rejected");
        };
        assert_eq!(abort.status, BatchStatus::Failed);
        let deferred = abort.deferred_command.unwrap();
        assert_eq!(deferred, new_first);

        let replacement = read_structured_submission(
            &PresentationGenerations::default(),
            deferred,
            &receiver,
            &TtsState::default(),
        )
        .unwrap();
        assert!(matches!(
            replacement,
            StructuredSubmissionRead::Prepared(presentation)
                if presentation.generation == 16
                    && presentation.timeline.dispatch_id == 38
        ));
    }

    #[test]
    fn stale_complete_multipart_submission_is_cancelled() {
        let mut commands = multipart_commands(17, 39).into_iter();
        let first = commands.next().unwrap();
        let second = commands.next().unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(format!(
                "emacsvox_timeline_part {}",
                second.args.unwrap()
            )))
            .unwrap();
        let mut generations = PresentationGenerations::default();
        generations.commit(17);

        let read = read_structured_submission(&generations, first, &receiver, &TtsState::default())
            .unwrap();

        assert!(matches!(
            read,
            StructuredSubmissionRead::Aborted(AbortedStructuredSubmission {
                generation: 17,
                dispatch_id: 39,
                status: BatchStatus::Cancelled,
                deferred_command: None,
                input_closed: false,
            })
        ));
    }
}
