//! Playback marker event preparation and asynchronous stdout delivery.

use omnivox_audio::{
    AudioBuffer, AudioControl, AudioError, CancellationToken, PlaybackCue, PlaybackTicket,
    ProgressivePlaybackProducer, StreamType,
};
use omnivox_core::timeline::TimelineActionId;
use omnivox_tts::contracts::{AcssDimension, PhysicalVoiceId, PostSynthesisDimension};
use omnivox_tts::marker_protocol::{
    format_marker_event, MarkerEvent, MarkerEventEnvelope, MARKER_PROTOCOL_VERSION,
    TIMELINE_EVENT_PROTOCOL_VERSION,
};
use omnivox_tts::{AnchorResolution, SynthesisMarker};
use std::cell::Cell;
use std::io::{self, Write};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

use crate::lifecycle::RequestLifecycle;

const MAX_QUEUED_MARKER_EVENTS: usize = 8 * 1024;
const MAX_QUEUED_MARKER_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROGRESSIVE_MARKERS: usize = 4096;
const MARKER_RESERVATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy)]
struct MarkerReporterLimits {
    max_events: usize,
    max_bytes: usize,
}

const MARKER_REPORTER_LIMITS: MarkerReporterLimits = MarkerReporterLimits {
    // One helper result may contain 4,096 markers. Timeline diagnostics and
    // semantic events share this budget without excluding a legal utterance.
    max_events: MAX_QUEUED_MARKER_EVENTS,
    // Match the server's bounded aggregate-presentation payload budget.
    max_bytes: MAX_QUEUED_MARKER_BYTES,
};

enum MarkerReporterMessage {
    Event(ReservedMarkerRecord),
    Terminal(ReservedTerminalRecord),
}

#[derive(Default)]
struct MarkerReporterCapacityState {
    events: usize,
    bytes: usize,
    terminal_in_flight: bool,
    closed: bool,
}

struct MarkerReporterCapacity {
    limits: MarkerReporterLimits,
    state: Mutex<MarkerReporterCapacityState>,
    available: Condvar,
}

impl MarkerReporterCapacity {
    fn new(limits: MarkerReporterLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(MarkerReporterCapacityState::default()),
            available: Condvar::new(),
        }
    }

    fn reserve_marker_records<P>(
        self: &Arc<Self>,
        records: Vec<String>,
        keep_waiting: &P,
    ) -> Result<Option<Vec<ReservedMarkerRecord>>, AudioError>
    where
        P: Fn() -> bool,
    {
        let event_count = records.len();
        let byte_count = records.iter().try_fold(0usize, |total, record| {
            total.checked_add(record.len()).ok_or_else(|| {
                AudioError::InvalidFormat("marker reporter batch byte count overflowed".to_owned())
            })
        })?;
        if event_count > self.limits.max_events {
            return Err(AudioError::InvalidFormat(format!(
                "marker reporter batch contains {event_count} events; limit is {}",
                self.limits.max_events
            )));
        }
        if byte_count > self.limits.max_bytes {
            return Err(AudioError::InvalidFormat(format!(
                "marker reporter batch contains {byte_count} encoded bytes; limit is {}",
                self.limits.max_bytes
            )));
        }

        let mut state = self.state.lock().unwrap();
        loop {
            if state.closed {
                return Err(AudioError::PlaybackError(
                    "marker reporter stopped before playback was queued".to_owned(),
                ));
            }
            if !keep_waiting() {
                return Ok(None);
            }
            let events_fit = state.events <= self.limits.max_events - event_count;
            let bytes_fit = state.bytes <= self.limits.max_bytes - byte_count;
            if events_fit && bytes_fit {
                state.events += event_count;
                state.bytes += byte_count;
                break;
            }
            (state, _) = self
                .available
                .wait_timeout(state, MARKER_RESERVATION_POLL_INTERVAL)
                .unwrap();
        }
        drop(state);

        Ok(Some(
            records
                .into_iter()
                .map(|record| ReservedMarkerRecord {
                    bytes: record.len(),
                    record,
                    capacity: self.clone(),
                })
                .collect(),
        ))
    }

    fn reserve_terminal(self: &Arc<Self>, record: String) -> Option<ReservedTerminalRecord> {
        let mut state = self.state.lock().unwrap();
        while state.terminal_in_flight && !state.closed {
            state = self.available.wait(state).unwrap();
        }
        if state.closed {
            return None;
        }
        state.terminal_in_flight = true;
        drop(state);
        Some(ReservedTerminalRecord {
            record,
            capacity: self.clone(),
        })
    }

    fn release_marker(&self, bytes: usize) {
        let mut state = self.state.lock().unwrap();
        state.events -= 1;
        state.bytes -= bytes;
        self.available.notify_all();
    }

    fn release_terminal(&self) {
        let mut state = self.state.lock().unwrap();
        state.terminal_in_flight = false;
        self.available.notify_all();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        self.available.notify_all();
    }
}

struct ReservedMarkerRecord {
    record: String,
    bytes: usize,
    capacity: Arc<MarkerReporterCapacity>,
}

impl Drop for ReservedMarkerRecord {
    fn drop(&mut self) {
        self.capacity.release_marker(self.bytes);
    }
}

struct ReservedTerminalRecord {
    record: String,
    capacity: Arc<MarkerReporterCapacity>,
}

impl Drop for ReservedTerminalRecord {
    fn drop(&mut self) {
        self.capacity.release_terminal();
    }
}

struct MarkerReporterCloseGuard(Arc<MarkerReporterCapacity>);

impl Drop for MarkerReporterCloseGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Marker output whose audio-thread event path is pre-reserved and nonblocking.
///
/// Terminal emission is allowed to backpressure its dedicated non-audio
/// producer so completion records are never discarded under saturation.
#[derive(Clone)]
pub struct MarkerEventOutput {
    sender: mpsc::SyncSender<MarkerReporterMessage>,
    capacity: Arc<MarkerReporterCapacity>,
}

impl MarkerEventOutput {
    fn format_events(
        &self,
        events: &[Arc<MarkerEventEnvelope>],
    ) -> Result<Vec<String>, AudioError> {
        if events.len() > self.capacity.limits.max_events {
            return Err(AudioError::InvalidFormat(format!(
                "marker reporter batch contains {} events; limit is {}",
                events.len(),
                self.capacity.limits.max_events
            )));
        }
        let mut byte_count = 0usize;
        let mut records = Vec::with_capacity(events.len());
        for event in events {
            let record = format_marker_event(event).map_err(|error| {
                AudioError::InvalidFormat(format!("could not encode playback marker: {error}"))
            })?;
            byte_count = byte_count.checked_add(record.len()).ok_or_else(|| {
                AudioError::InvalidFormat("marker reporter batch byte count overflowed".to_owned())
            })?;
            if byte_count > self.capacity.limits.max_bytes {
                return Err(AudioError::InvalidFormat(format!(
                    "marker reporter batch contains {byte_count} encoded bytes; limit is {}",
                    self.capacity.limits.max_bytes
                )));
            }
            records.push(record);
        }
        Ok(records)
    }

    fn reserve_marker_records<P>(
        &self,
        records: Vec<String>,
        keep_waiting: &P,
    ) -> Result<Option<Vec<ReservedMarkerRecord>>, AudioError>
    where
        P: Fn() -> bool,
    {
        self.capacity.reserve_marker_records(records, keep_waiting)
    }

    fn emit(&self, event: ReservedMarkerRecord) {
        match self.sender.try_send(MarkerReporterMessage::Event(event)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(message)) => {
                warn!(
                    "Reserved marker event reached an unexpectedly full reporter channel; applying lossless backpressure"
                );
                let _ = self.sender.send(message);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                warn!("Marker reporter stopped before a reserved marker was written")
            }
        }
    }

    /// Queue one terminal record behind all marker events already emitted.
    pub(crate) fn emit_terminal(&self, record: String) -> bool {
        let Some(record) = self.capacity.reserve_terminal(record) else {
            return false;
        };
        self.sender
            .send(MarkerReporterMessage::Terminal(record))
            .is_ok()
    }

    #[cfg(test)]
    pub(crate) fn emit_test_record(&self, record: &str) -> Result<(), AudioError> {
        let mut records = self
            .reserve_marker_records(vec![record.to_owned()], &|| true)?
            .expect("test marker reservation should not be cancelled");
        self.emit(records.pop().expect("test marker record should be present"));
        Ok(())
    }
}

/// Spawn the single writer that serializes marker events to stdout.
pub fn spawn_marker_event_reporter() -> (MarkerEventOutput, std::thread::JoinHandle<()>) {
    let (output, receiver) = marker_reporter_channel(MARKER_REPORTER_LIMITS);
    let capacity = output.capacity.clone();
    let handle = std::thread::Builder::new()
        .name("omnivox-marker-reporter".to_owned())
        .spawn(move || {
            run_marker_event_reporter(receiver, capacity, |message| {
                // Other server threads also write protocol records to stdout.
                // Keep this lock scoped to one complete reporter record so an
                // idle reporter cannot block control responses indefinitely.
                let stdout = io::stdout();
                write_marker_reporter_message(&mut stdout.lock(), message);
            });
        })
        .expect("Failed to spawn marker event reporter thread");
    (output, handle)
}

#[cfg(test)]
pub(crate) fn spawn_marker_event_reporter_with_writer<W>(
    writer: W,
) -> (MarkerEventOutput, std::thread::JoinHandle<()>)
where
    W: Write + Send + 'static,
{
    spawn_marker_event_reporter_with_limits(writer, MARKER_REPORTER_LIMITS)
}

#[cfg(test)]
fn spawn_marker_event_reporter_with_limits<W>(
    writer: W,
    limits: MarkerReporterLimits,
) -> (MarkerEventOutput, std::thread::JoinHandle<()>)
where
    W: Write + Send + 'static,
{
    let (output, receiver) = marker_reporter_channel(limits);
    let capacity = output.capacity.clone();
    let handle = std::thread::spawn(move || {
        let mut writer = writer;
        run_marker_event_reporter(receiver, capacity, |message| {
            write_marker_reporter_message(&mut writer, message);
        });
    });
    (output, handle)
}

fn marker_reporter_channel(
    limits: MarkerReporterLimits,
) -> (MarkerEventOutput, mpsc::Receiver<MarkerReporterMessage>) {
    let capacity = Arc::new(MarkerReporterCapacity::new(limits));
    // The extra slot is exclusively protected by the terminal reservation.
    // Marker reservations therefore make audio-thread try_send infallible
    // while one lossless terminal record is pending.
    let (sender, receiver) = mpsc::sync_channel(limits.max_events + 1);
    (MarkerEventOutput { sender, capacity }, receiver)
}

fn run_marker_event_reporter<F>(
    receiver: mpsc::Receiver<MarkerReporterMessage>,
    capacity: Arc<MarkerReporterCapacity>,
    mut report: F,
) where
    F: FnMut(&MarkerReporterMessage),
{
    let _close_guard = MarkerReporterCloseGuard(capacity);
    for message in receiver {
        report(&message);
    }
}

fn write_marker_reporter_message<W: Write>(writer: &mut W, message: &MarkerReporterMessage) {
    let (record, kind) = match message {
        MarkerReporterMessage::Event(event) => (event.record.as_str(), "marker event"),
        MarkerReporterMessage::Terminal(terminal) => {
            (terminal.record.as_str(), "playback terminal")
        }
    };
    if let Err(error) = writeln!(writer, "{}", record).and_then(|_| writer.flush()) {
        warn!("Could not write {}: {}", kind, error);
    }
}

/// Per-dispatch sequence and route context used while synthesis queues chunks.
pub struct MarkerDispatchContext {
    dispatch_id: u64,
    protocol_version: u32,
    next_sequence: Cell<u64>,
    next_utterance_id: Cell<u64>,
    output: MarkerEventOutput,
    lifecycle: Option<RequestLifecycle>,
}

impl MarkerDispatchContext {
    pub fn new(dispatch_id: u64, output: MarkerEventOutput) -> Self {
        Self {
            dispatch_id,
            protocol_version: MARKER_PROTOCOL_VERSION,
            next_sequence: Cell::new(0),
            next_utterance_id: Cell::new(0),
            output,
            lifecycle: None,
        }
    }

    /// Create a dispatch capable of emitting playback-bound semantic actions.
    #[allow(dead_code)] // Activated by the structured timeline transport slice.
    pub fn with_timeline_events(dispatch_id: u64, output: MarkerEventOutput) -> Self {
        Self {
            dispatch_id,
            protocol_version: TIMELINE_EVENT_PROTOCOL_VERSION,
            next_sequence: Cell::new(0),
            next_utterance_id: Cell::new(0),
            output,
            lifecycle: None,
        }
    }

    pub fn with_lifecycle(mut self, lifecycle: RequestLifecycle) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn supports_timeline_events(&self) -> bool {
        self.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_utterance(
        &self,
        text: &str,
        engine_id: &str,
        actual_voice: Option<&PhysicalVoiceId>,
        logical_voice_id: Option<&str>,
        sample_rate: u32,
        frame_count: usize,
        markers: &[SynthesisMarker],
        semantic_events: &[PlaybackSemanticEvent],
    ) -> PreparedMarkerPlayback {
        self.prepare_timeline_utterance(
            text,
            engine_id,
            actual_voice,
            logical_voice_id,
            sample_rate,
            frame_count,
            markers,
            semantic_events,
            &[],
            &[],
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_timeline_utterance(
        &self,
        text: &str,
        engine_id: &str,
        actual_voice: Option<&PhysicalVoiceId>,
        logical_voice_id: Option<&str>,
        sample_rate: u32,
        frame_count: usize,
        markers: &[SynthesisMarker],
        semantic_events: &[PlaybackSemanticEvent],
        resolutions: &[PlaybackTimelineResolution],
        degraded_acss: &[AcssDimension],
        degraded_effects: &[PostSynthesisDimension],
    ) -> PreparedMarkerPlayback {
        assert!(
            (semantic_events.is_empty()
                && resolutions.is_empty()
                && degraded_acss.is_empty()
                && degraded_effects.is_empty())
                || self.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION,
            "timeline playback events require a version 2 dispatch"
        );
        let utterance_id = increment(&self.next_utterance_id);
        let diagnostic_count = resolutions.len()
            + usize::from(!degraded_acss.is_empty() || !degraded_effects.is_empty());
        let mut events =
            Vec::with_capacity(markers.len() + semantic_events.len() + diagnostic_count + 1);
        let mut cues =
            Vec::with_capacity(markers.len() + semantic_events.len() + diagnostic_count + 1);
        push_event(
            &mut events,
            &mut cues,
            0,
            MarkerEventEnvelope {
                protocol_version: self.protocol_version,
                dispatch_id: self.dispatch_id,
                sequence: increment(&self.next_sequence),
                event: MarkerEvent::UtteranceStarted {
                    utterance_id,
                    text: text.to_owned(),
                    engine_id: engine_id.to_owned(),
                    actual_voice: actual_voice.cloned(),
                    logical_voice_id: logical_voice_id.map(str::to_owned),
                    sample_rate,
                    frame_count: frame_count as u64,
                },
            },
        );
        for resolution in resolutions {
            push_event(
                &mut events,
                &mut cues,
                0,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event: MarkerEvent::TimelineActionResolved {
                        utterance_id,
                        action_id: resolution.action_id.as_str().to_owned(),
                        resolution: resolution.resolution,
                    },
                },
            );
        }
        if !degraded_acss.is_empty() || !degraded_effects.is_empty() {
            push_event(
                &mut events,
                &mut cues,
                0,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event: MarkerEvent::TimelineStyleDegraded {
                        utterance_id,
                        degraded_acss: degraded_acss.to_vec(),
                        degraded_effects: degraded_effects.to_vec(),
                    },
                },
            );
        }

        let mut pending = markers
            .iter()
            .cloned()
            .enumerate()
            .map(|(order, marker)| {
                (
                    marker.frame_offset,
                    order,
                    MarkerEvent::MarkerReached {
                        utterance_id,
                        marker,
                    },
                )
            })
            .chain(semantic_events.iter().enumerate().map(|(index, event)| {
                (
                    event.frame_offset,
                    markers.len() + index,
                    MarkerEvent::SemanticEventReached {
                        utterance_id,
                        action_id: event.action_id.as_str().to_owned(),
                    },
                )
            }))
            .collect::<Vec<_>>();
        pending.sort_by_key(|(frame_offset, order, _)| (*frame_offset, *order));
        for (frame_offset, _, event) in pending {
            push_event(
                &mut events,
                &mut cues,
                frame_offset,
                MarkerEventEnvelope {
                    protocol_version: self.protocol_version,
                    dispatch_id: self.dispatch_id,
                    sequence: increment(&self.next_sequence),
                    event,
                },
            );
        }

        PreparedMarkerPlayback {
            cues,
            events: Arc::new(events),
            output: self.output.clone(),
            lifecycle: self.lifecycle.clone(),
            dispatch_id: self.dispatch_id,
            protocol_version: self.protocol_version,
            utterance_id,
        }
    }
}

/// An opaque semantic action already mapped to the mixed output clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSemanticEvent {
    pub action_id: TimelineActionId,
    pub frame_offset: u64,
}

/// One requested action and the placement grade realized by synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackTimelineResolution {
    pub action_id: TimelineActionId,
    pub resolution: AnchorResolution,
}

pub struct PreparedMarkerPlayback {
    cues: Vec<PlaybackCue>,
    events: Arc<Vec<Arc<MarkerEventEnvelope>>>,
    output: MarkerEventOutput,
    lifecycle: Option<RequestLifecycle>,
    dispatch_id: u64,
    protocol_version: u32,
    utterance_id: u64,
}

/// Producer-side state for adding pre-reserved marker records to a progressive
/// playback source before audio crosses their frame offsets.
pub(crate) struct ProgressiveMarkerPublisher {
    dispatch_id: u64,
    protocol_version: u32,
    utterance_id: u64,
    events: Arc<Mutex<Vec<Option<ReservedMarkerRecord>>>>,
    output: MarkerEventOutput,
    marker_count: usize,
    last_frame_offset: Option<u64>,
}

impl ProgressiveMarkerPublisher {
    pub(crate) fn push_markers<P>(
        &mut self,
        marker_dispatch: &MarkerDispatchContext,
        producer: &mut ProgressivePlaybackProducer,
        markers: Vec<SynthesisMarker>,
        predicate: P,
    ) -> Result<bool, AudioError>
    where
        P: Fn() -> bool,
    {
        if markers.is_empty() {
            return Err(AudioError::InvalidFormat(
                "progressive marker batch is empty".to_owned(),
            ));
        }
        if marker_dispatch.dispatch_id != self.dispatch_id
            || marker_dispatch.protocol_version != self.protocol_version
        {
            return Err(AudioError::InvalidFormat(
                "progressive markers used a different dispatch context".to_owned(),
            ));
        }
        self.marker_count = self
            .marker_count
            .checked_add(markers.len())
            .filter(|count| *count <= MAX_PROGRESSIVE_MARKERS)
            .ok_or_else(|| {
                AudioError::InvalidFormat(format!(
                    "progressive utterance exceeds the {MAX_PROGRESSIVE_MARKERS}-marker limit"
                ))
            })?;

        let mut envelopes = Vec::with_capacity(markers.len());
        let mut cues = Vec::with_capacity(markers.len());
        let first_identifier = self.events.lock().unwrap().len() as u64;
        for (index, marker) in markers.into_iter().enumerate() {
            if self
                .last_frame_offset
                .is_some_and(|offset| marker.frame_offset < offset)
            {
                return Err(AudioError::InvalidFormat(
                    "progressive markers are out of playback order".to_owned(),
                ));
            }
            self.last_frame_offset = Some(marker.frame_offset);
            envelopes.push(Arc::new(MarkerEventEnvelope {
                protocol_version: self.protocol_version,
                dispatch_id: self.dispatch_id,
                sequence: increment(&marker_dispatch.next_sequence),
                event: MarkerEvent::MarkerReached {
                    utterance_id: self.utterance_id,
                    marker,
                },
            }));
            cues.push(PlaybackCue {
                frame_offset: self.last_frame_offset.unwrap(),
                identifier: first_identifier.checked_add(index as u64).ok_or_else(|| {
                    AudioError::InvalidFormat("progressive marker identifier overflowed".to_owned())
                })?,
            });
        }
        let records = self.output.format_events(&envelopes)?;
        let Some(records) = self.output.reserve_marker_records(records, &predicate)? else {
            return Ok(false);
        };
        self.events
            .lock()
            .unwrap()
            .extend(records.into_iter().map(Some));
        producer.push_cues(cues)?;
        Ok(true)
    }
}

impl PreparedMarkerPlayback {
    pub fn queue_if<F>(
        self,
        control: &AudioControl,
        buffer: &AudioBuffer,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: Fn() -> bool,
    {
        self.queue_if_with_cancellation(control, buffer, None, predicate)
    }

    pub fn queue_cancellable_if<F>(
        self,
        control: &AudioControl,
        buffer: &AudioBuffer,
        cancellation: CancellationToken,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: Fn() -> bool,
    {
        self.queue_if_with_cancellation(control, buffer, Some(cancellation), predicate)
    }

    /// Queue a progressive speech source whose events and initial cues are
    /// already known while its final audio length is not.
    pub fn queue_progressive_cancellable_if<F>(
        self,
        control: &AudioControl,
        cancellation: CancellationToken,
        predicate: F,
    ) -> Result<
        Option<(
            ProgressivePlaybackProducer,
            PlaybackTicket,
            ProgressiveMarkerPublisher,
        )>,
        AudioError,
    >
    where
        F: Fn() -> bool,
    {
        let records = self.output.format_events(&self.events)?;
        let Some(events) = self.output.reserve_marker_records(records, &predicate)? else {
            return Ok(None);
        };
        let events = Arc::new(Mutex::new(events.into_iter().map(Some).collect::<Vec<_>>()));
        let callback_events = Arc::clone(&events);
        let output = self.output;
        let callback_output = output.clone();
        let lifecycle = self.lifecycle.clone();
        let on_cue = move |cue: PlaybackCue| {
            if cue.identifier == 0 {
                if let Some(lifecycle) = &lifecycle {
                    lifecycle.record_mixer_source_started();
                }
            }
            let event = callback_events
                .lock()
                .unwrap()
                .get_mut(cue.identifier as usize)
                .and_then(Option::take);
            if let Some(event) = event {
                callback_output.emit(event);
            }
        };
        let queue_attempted_at = Instant::now();
        let result = control.queue_progressive_speech_with_cue_callback_cancellable_if(
            on_cue,
            cancellation,
            predicate,
        );
        if matches!(result, Ok(Some(_))) {
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.record_audio_queued_at(queue_attempted_at);
            }
        }
        let Some((mut producer, ticket)) = result? else {
            return Ok(None);
        };
        producer.push_cues(self.cues)?;
        Ok(Some((
            producer,
            ticket,
            ProgressiveMarkerPublisher {
                dispatch_id: self.dispatch_id,
                protocol_version: self.protocol_version,
                utterance_id: self.utterance_id,
                events,
                output,
                marker_count: 0,
                last_frame_offset: None,
            },
        )))
    }

    fn queue_if_with_cancellation<F>(
        self,
        control: &AudioControl,
        buffer: &AudioBuffer,
        cancellation: Option<CancellationToken>,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: Fn() -> bool,
    {
        let records = self.output.format_events(&self.events)?;
        let Some(events) = self.output.reserve_marker_records(records, &predicate)? else {
            return Ok(None);
        };
        let mut events = events.into_iter().map(Some).collect::<Vec<_>>();
        let output = self.output;
        let lifecycle = self.lifecycle.clone();
        let on_cue = move |cue: PlaybackCue| {
            if cue.identifier == 0 {
                if let Some(lifecycle) = &lifecycle {
                    lifecycle.record_mixer_source_started();
                }
            }
            if let Some(event) = events
                .get_mut(cue.identifier as usize)
                .and_then(Option::take)
            {
                output.emit(event);
            }
        };
        let queue_attempted_at = Instant::now();
        let result = if let Some(cancellation) = cancellation {
            control.queue_tracked_with_cue_callback_cancellable_if(
                StreamType::Speech,
                buffer,
                self.cues,
                on_cue,
                cancellation,
                predicate,
            )
        } else {
            control.queue_tracked_with_cue_callback_if(
                StreamType::Speech,
                buffer,
                self.cues,
                on_cue,
                predicate,
            )
        };
        if matches!(result, Ok(Some(_))) {
            if let Some(lifecycle) = &self.lifecycle {
                lifecycle.record_audio_queued_at(queue_attempted_at);
            }
        }
        result
    }
}

fn increment(counter: &Cell<u64>) -> u64 {
    let next = counter
        .get()
        .checked_add(1)
        .expect("marker sequence overflow");
    counter.set(next);
    next
}

fn push_event(
    events: &mut Vec<Arc<MarkerEventEnvelope>>,
    cues: &mut Vec<PlaybackCue>,
    frame_offset: u64,
    event: MarkerEventEnvelope,
) {
    let identifier = events.len() as u64;
    events.push(Arc::new(event));
    cues.push(PlaybackCue {
        frame_offset,
        identifier,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_audio::{AudioBackend, AudioStreams, PlaybackStatus};
    use omnivox_tts::marker_protocol::{decode_marker_event, MARKER_EVENT_PREFIX};
    use omnivox_tts::SynthesisMarkerKind;

    #[derive(Clone, Default)]
    struct RecordingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BlockingWriter {
        writer: RecordingWriter,
        started: mpsc::SyncSender<()>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Write for BlockingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let _ = self.started.try_send(());
            let (released, available) = &*self.release;
            let mut released = released.lock().unwrap();
            while !*released {
                released = available.wait(released).unwrap();
            }
            drop(released);
            self.writer.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.writer.flush()
        }
    }

    fn unreported_output() -> MarkerEventOutput {
        marker_reporter_channel(MARKER_REPORTER_LIMITS).0
    }

    fn marker(frame_offset: u64, value: &str) -> SynthesisMarker {
        SynthesisMarker {
            kind: SynthesisMarkerKind::Word,
            frame_offset,
            text_start: None,
            text_length: None,
            value: Some(value.to_owned()),
        }
    }

    #[test]
    fn prepares_started_event_then_stably_sorted_markers() {
        let context = MarkerDispatchContext::new(73, unreported_output());
        let prepared = context.prepare_utterance(
            "hello world",
            "helper",
            Some(&PhysicalVoiceId::new("helper", "paul")),
            Some("source-code"),
            44100,
            100,
            &[
                marker(50, "second"),
                marker(10, "first"),
                marker(10, "same"),
            ],
            &[],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 10, 10, 50]
        );
        assert_eq!(
            prepared
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert!(matches!(
            prepared.events[0].event,
            MarkerEvent::UtteranceStarted {
                ref text,
                ref engine_id,
                ref logical_voice_id,
                ..
            } if text == "hello world"
                && engine_id == "helper"
                && logical_voice_id.as_deref() == Some("source-code")
        ));
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::MarkerReached {
                ref marker,
                ..
            } if marker.value.as_deref() == Some("first")
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::MarkerReached {
                ref marker,
                ..
            } if marker.value.as_deref() == Some("same")
        ));
    }

    #[test]
    fn v2_semantic_events_are_stably_merged_at_playback_frames() {
        let context = MarkerDispatchContext::with_timeline_events(91, unreported_output());
        let prepared = context.prepare_utterance(
            "hello",
            "helper",
            None,
            None,
            44100,
            100,
            &[marker(20, "word")],
            &[
                PlaybackSemanticEvent {
                    action_id: TimelineActionId::new("same-frame").unwrap(),
                    frame_offset: 20,
                },
                PlaybackSemanticEvent {
                    action_id: TimelineActionId::new("earlier").unwrap(),
                    frame_offset: 10,
                },
            ],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 10, 20, 20]
        );
        assert!(prepared
            .events
            .iter()
            .all(|event| { event.protocol_version == TIMELINE_EVENT_PROTOCOL_VERSION }));
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::SemanticEventReached { ref action_id, .. }
                if action_id == "earlier"
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::MarkerReached { .. }
        ));
        assert!(matches!(
            prepared.events[3].event,
            MarkerEvent::SemanticEventReached { ref action_id, .. }
                if action_id == "same-frame"
        ));
    }

    #[test]
    fn v2_reports_anchor_and_style_degradation_at_utterance_start() {
        let context = MarkerDispatchContext::with_timeline_events(92, unreported_output());
        let prepared = context.prepare_timeline_utterance(
            "hello",
            "helper",
            None,
            Some("comment"),
            44100,
            100,
            &[],
            &[],
            &[PlaybackTimelineResolution {
                action_id: TimelineActionId::new("cue").unwrap(),
                resolution: AnchorResolution::WordBoundary,
            }],
            &[AcssDimension::Richness],
            &[PostSynthesisDimension::Echo],
        );

        assert_eq!(
            prepared
                .cues
                .iter()
                .map(|cue| cue.frame_offset)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(matches!(
            prepared.events[1].event,
            MarkerEvent::TimelineActionResolved {
                ref action_id,
                resolution: AnchorResolution::WordBoundary,
                ..
            } if action_id == "cue"
        ));
        assert!(matches!(
            prepared.events[2].event,
            MarkerEvent::TimelineStyleDegraded {
                ref degraded_acss,
                ref degraded_effects,
                ..
            } if degraded_acss == &[AcssDimension::Richness]
                && degraded_effects == &[PostSynthesisDimension::Echo]
        ));
    }

    #[test]
    fn progressive_markers_are_reserved_before_their_audio_and_reach_the_reporter() {
        let writer = RecordingWriter::default();
        let written = writer.bytes.clone();
        let (output, reporter) = spawn_marker_event_reporter_with_writer(writer);
        let context = MarkerDispatchContext::new(93, output);
        let prepared = context.prepare_utterance("hello", "helper", None, None, 44100, 0, &[], &[]);
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let (mut producer, ticket, mut publisher) = prepared
            .queue_progressive_cancellable_if(&control, CancellationToken::new(), || true)
            .unwrap()
            .unwrap();

        assert!(publisher
            .push_markers(&context, &mut producer, vec![marker(1, "hello")], || true)
            .unwrap());
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]))
            .unwrap();
        producer.finish().unwrap();

        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        control.drain();
        drop(publisher);
        drop(context);
        drop(control);
        drop(streams);
        reporter.join().unwrap();

        let records = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        let events = records
            .lines()
            .map(|record| {
                let payload = record
                    .strip_prefix(MARKER_EVENT_PREFIX)
                    .unwrap()
                    .trim_start();
                decode_marker_event(payload).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
        assert!(matches!(
            events[1].event,
            MarkerEvent::MarkerReached {
                ref marker,
                utterance_id: 1,
            } if marker.frame_offset == 1 && marker.value.as_deref() == Some("hello")
        ));
    }

    #[test]
    fn blocked_writer_keeps_reserved_marker_memory_bounded() {
        let limits = MarkerReporterLimits {
            max_events: 2,
            max_bytes: 6,
        };
        let recording = RecordingWriter::default();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let writer = BlockingWriter {
            writer: recording,
            started: started_tx,
            release: release.clone(),
        };
        let (output, reporter) = spawn_marker_event_reporter_with_limits(writer, limits);
        let mut reserved = output
            .reserve_marker_records(vec!["one".to_owned(), "two".to_owned()], &|| true)
            .unwrap()
            .unwrap();
        output.emit(reserved.remove(0));
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        output.emit(reserved.remove(0));

        let waiting_output = output.clone();
        let (waiting_tx, waiting_rx) = mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let reserved = waiting_output
                .reserve_marker_records(vec!["x".to_owned()], &|| true)
                .unwrap()
                .unwrap();
            waiting_tx.send(reserved).unwrap();
        });

        assert!(matches!(
            waiting_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        {
            let state = output.capacity.state.lock().unwrap();
            assert_eq!(state.events, limits.max_events);
            assert_eq!(state.bytes, limits.max_bytes);
        }

        let (released, available) = &*release;
        *released.lock().unwrap() = true;
        available.notify_all();
        drop(waiting_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        waiter.join().unwrap();
        drop(output);
        reporter.join().unwrap();
    }

    #[test]
    fn cancelled_or_oversized_marker_reservations_never_enter_the_queue() {
        let limits = MarkerReporterLimits {
            max_events: 1,
            max_bytes: 3,
        };
        let (output, _receiver) = marker_reporter_channel(limits);
        let held = output
            .reserve_marker_records(vec!["one".to_owned()], &|| true)
            .unwrap()
            .unwrap();

        assert!(output
            .reserve_marker_records(vec!["two".to_owned()], &|| false)
            .unwrap()
            .is_none());
        assert!(matches!(
            output.reserve_marker_records(vec!["four".to_owned()], &|| true),
            Err(AudioError::InvalidFormat(_))
        ));
        {
            let state = output.capacity.state.lock().unwrap();
            assert_eq!(state.events, 1);
            assert_eq!(state.bytes, 3);
        }

        drop(held);
        let state = output.capacity.state.lock().unwrap();
        assert_eq!(state.events, 0);
        assert_eq!(state.bytes, 0);
    }

    #[test]
    fn production_reporter_releases_stdout_between_records() {
        let (output, reporter) = spawn_marker_event_reporter();
        output.emit_test_record("marker-lock-probe").unwrap();

        let state = output.capacity.state.lock().unwrap();
        let (state, wait) = output
            .capacity
            .available
            .wait_timeout_while(state, Duration::from_secs(2), |state| state.events != 0)
            .unwrap();
        assert!(
            !wait.timed_out(),
            "reporter did not consume the probe record"
        );
        drop(state);

        let (acquired_tx, acquired_rx) = mpsc::sync_channel(1);
        let contender = std::thread::spawn(move || {
            let stdout = io::stdout();
            let _stdout = stdout.lock();
            let _ = acquired_tx.send(());
        });
        acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("idle marker reporter retained stdout's lock");
        contender.join().unwrap();

        drop(output);
        reporter.join().unwrap();
    }

    #[test]
    fn reporter_serializes_markers_before_one_terminal_record() {
        let writer = RecordingWriter::default();
        let written = writer.bytes.clone();
        let (output, reporter) = spawn_marker_event_reporter_with_writer(writer);

        output.emit_test_record("marker-one").unwrap();
        output.emit_test_record("marker-two").unwrap();
        assert!(output.emit_terminal("terminal".to_owned()));
        drop(output);
        reporter.join().unwrap();

        assert_eq!(
            String::from_utf8(written.lock().unwrap().clone()).unwrap(),
            "marker-one\nmarker-two\nterminal\n"
        );
    }
}
