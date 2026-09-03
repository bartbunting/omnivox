//! Audio Output
//!
//! Wraps rodio for audio playback. Provides both single-shot playback
//! (`AudioOutput`) and concurrent multi-stream playback (`AudioStreams`).
//!
//! The key type for multi-threaded use is `AudioControl` -- a `Send + Sync`
//! handle to the three audio sinks. In device mode, `AudioStreams` owns the
//! `OutputStream` drop guard (which is `!Send` on some platforms) and should
//! stay on the thread where it was created. In null mode, it owns the source
//! consumer workers. `AudioControl` can be cloned via `AudioStreams::control()`
//! and sent to other threads (e.g. a synthesis worker) to queue or stop audio
//! independently.

use crate::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use crate::{AudioError, CancellationToken};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::debug;

const SPEECH_STOP_FADE_MILLISECONDS: usize = 3;
const SPEECH_STOP_FADE_FRAMES: usize = SAMPLE_RATE as usize * SPEECH_STOP_FADE_MILLISECONDS / 1000;
const TONE_STOP_FADE_MILLISECONDS: usize = 5;
const TONE_STOP_FADE_FRAMES: usize = SAMPLE_RATE as usize * TONE_STOP_FADE_MILLISECONDS / 1000;
const NULL_AUDIO_POLL_SAMPLES: usize = 1024;
const PROGRESSIVE_PLAYBACK_CAPACITY: usize = 4;
const PROGRESSIVE_PLAYBACK_PREBUFFER_WINDOWS: usize = 3;
const PROGRESSIVE_PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Selects whether streams play through the default device or discard samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackend {
    /// Play through the default system audio device in real time.
    Device,
    /// Consume every source as quickly as possible without opening a device.
    Null,
}

/// Which audio stream to route to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// Speech output (TTS, letters, silence gaps). Serialized.
    Speech,
    /// Tone beeps. Serialized within stream, concurrent with others.
    Tone,
    /// Audio icons / sound files. Serialized within stream, concurrent with others.
    Sound,
}

fn stream_index(stream: StreamType) -> usize {
    match stream {
        StreamType::Speech => 0,
        StreamType::Tone => 1,
        StreamType::Sound => 2,
    }
}

/// Terminal state of one queued audio buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    /// The audio source was consumed to its natural end.
    Completed,
    /// The source was cleared or dropped before its natural end.
    Cancelled,
}

/// An opaque caller-defined cue attached to a canonical audio frame.
///
/// Cue identifiers let higher layers associate playback timing with metadata
/// without making the audio crate depend on TTS marker types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackCue {
    /// Zero-based frame boundary in the queued audio buffer.
    pub frame_offset: u64,
    /// Opaque identifier returned unchanged when the cue is reached.
    pub identifier: u64,
}

enum ProgressivePlaybackMessage {
    Audio {
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
    },
    Complete {
        cues: Vec<PlaybackCue>,
    },
}

/// Bounded producer for one progressively queued speech source.
///
/// Audio and cues are supplied in playback order. Cues are carried by the next
/// audio window rather than occupying bounded PCM capacity. Sending blocks
/// with a short cancellation-aware poll when the audio consumer is behind,
/// bounding memory without making request cancellation wait for earlier
/// queued speech.
pub struct ProgressivePlaybackProducer {
    sender: Option<SyncSender<ProgressivePlaybackMessage>>,
    request_cancellation: CancellationToken,
    stream_cancellation: CancellationToken,
    published_frames: u64,
    last_cue_offset: Option<u64>,
    pending_cues: Vec<PlaybackCue>,
    prebuffered_audio_windows: usize,
    pending_attachment: Option<ProgressivePlaybackAttachment>,
}

impl ProgressivePlaybackProducer {
    /// Publish one non-empty canonical PCM window.
    pub fn push_audio(&mut self, audio: AudioBuffer) -> Result<(), AudioError> {
        if audio.is_empty() {
            return Err(AudioError::InvalidFormat(
                "progressive playback audio window is empty".to_owned(),
            ));
        }
        let frames = audio.frame_count() as u64;
        let published_frames = self.published_frames.checked_add(frames).ok_or_else(|| {
            AudioError::InvalidFormat("progressive playback frame count overflowed".to_owned())
        })?;
        let cues = std::mem::take(&mut self.pending_cues);
        self.send(ProgressivePlaybackMessage::Audio {
            samples: audio.samples,
            cues,
        })?;
        self.published_frames = published_frames;
        self.prebuffered_audio_windows = self.prebuffered_audio_windows.saturating_add(1);
        self.attach_if_primed(false)?;
        Ok(())
    }

    /// Publish one non-empty, monotonically ordered cue batch.
    ///
    /// A cue must arrive before audio beyond its frame boundary is published.
    pub fn push_cues(&mut self, cues: Vec<PlaybackCue>) -> Result<(), AudioError> {
        if cues.is_empty() {
            return Err(AudioError::InvalidFormat(
                "progressive playback cue batch is empty".to_owned(),
            ));
        }
        let mut previous = self.last_cue_offset;
        for cue in &cues {
            if cue.frame_offset < self.published_frames {
                return Err(AudioError::InvalidFormat(format!(
                    "progressive playback cue {} at frame {} arrived after {} frames",
                    cue.identifier, cue.frame_offset, self.published_frames
                )));
            }
            if previous.is_some_and(|offset| cue.frame_offset < offset) {
                return Err(AudioError::InvalidFormat(
                    "progressive playback cues are out of order".to_owned(),
                ));
            }
            previous = Some(cue.frame_offset);
        }
        self.pending_cues.extend(cues);
        self.last_cue_offset = previous;
        Ok(())
    }

    /// Mark the source complete after its final PCM window and cue batch.
    pub fn finish(mut self) -> Result<(), AudioError> {
        if let Some(offset) = self
            .last_cue_offset
            .filter(|offset| *offset > self.published_frames)
        {
            return Err(AudioError::InvalidFormat(format!(
                "progressive playback cue at frame {offset} exceeds the {}-frame stream",
                self.published_frames
            )));
        }
        let cues = std::mem::take(&mut self.pending_cues);
        self.send(ProgressivePlaybackMessage::Complete { cues })?;
        self.attach_if_primed(true)?;
        self.sender = None;
        Ok(())
    }

    /// Number of canonical frames successfully handed to the bounded source.
    pub fn published_frames(&self) -> u64 {
        self.published_frames
    }

    fn send(&self, mut message: ProgressivePlaybackMessage) -> Result<(), AudioError> {
        let sender = self.sender.as_ref().ok_or_else(|| {
            AudioError::PlaybackError("progressive playback is already complete".to_owned())
        })?;
        loop {
            if self.request_cancellation.is_cancelled() || self.stream_cancellation.is_cancelled() {
                return Err(AudioError::PlaybackError(
                    "progressive playback was cancelled".to_owned(),
                ));
            }
            match sender.try_send(message) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    message = returned;
                    std::thread::sleep(PROGRESSIVE_PLAYBACK_POLL_INTERVAL);
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(AudioError::PlaybackError(
                        "progressive playback consumer stopped".to_owned(),
                    ));
                }
            }
        }
    }

    fn attach_if_primed(&mut self, force: bool) -> Result<(), AudioError> {
        if self.pending_attachment.is_none()
            || (!force && self.prebuffered_audio_windows < PROGRESSIVE_PLAYBACK_PREBUFFER_WINDOWS)
        {
            return Ok(());
        }
        if self.request_cancellation.is_cancelled() || self.stream_cancellation.is_cancelled() {
            self.pending_attachment = None;
            return Err(AudioError::PlaybackError(
                "progressive playback was cancelled before it was primed".to_owned(),
            ));
        }

        let mut attachment = self
            .pending_attachment
            .take()
            .expect("pending progressive source was checked");
        let stream = StreamType::Speech;
        let stream_index = stream_index(stream);
        let _gate = attachment.control.stream_gates[stream_index]
            .lock()
            .unwrap();
        if self.request_cancellation.is_cancelled()
            || self.stream_cancellation.is_cancelled()
            || attachment.control.schedule_generations[stream_index].load(Ordering::Acquire)
                != attachment.generation
        {
            return Err(AudioError::PlaybackError(
                "progressive playback was cancelled before it was primed".to_owned(),
            ));
        }

        let (sink, max_depth) = attachment.control.sink_and_max(stream);
        if sink.len() >= max_depth {
            debug!(
                "Stream {:?} at capacity ({}/{}), clearing backlog before primed playback",
                stream,
                sink.len(),
                max_depth
            );
            let replacement = CancellationToken::new();
            let previous = std::mem::replace(
                &mut *attachment.control.speech_stop_cancellation.lock().unwrap(),
                replacement.clone(),
            );
            previous.cancel();
            self.stream_cancellation = replacement.clone();
            attachment
                .source
                .as_mut()
                .expect("pending progressive source is attached once")
                .cancellation
                .stream = Some(replacement);
            sink.clear();
            sink.play();
        }
        sink.append(
            attachment
                .source
                .take()
                .expect("pending progressive source is attached once"),
        );
        sink.play();
        Ok(())
    }
}

struct ProgressivePlaybackAttachment {
    control: AudioControl,
    source: Option<ProgressivePlaybackSource>,
    generation: u64,
}

/// Cloneable acknowledgement for a queued or scheduled audio buffer.
#[derive(Clone)]
pub struct PlaybackTicket {
    state: Arc<PlaybackCompletionState>,
}

impl PlaybackTicket {
    /// Wait until playback completes naturally or is cancelled.
    pub fn wait(self) -> PlaybackStatus {
        let mut status = self.state.status.lock().unwrap();
        while status.is_none() {
            status = self.state.changed.wait(status).unwrap();
        }
        status.unwrap_or(PlaybackStatus::Cancelled)
    }
}

struct PlaybackCompletionState {
    status: Mutex<Option<PlaybackStatus>>,
    changed: Condvar,
}

struct PlaybackCompletion {
    state: Arc<PlaybackCompletionState>,
    reported: bool,
}

impl PlaybackCompletion {
    fn pair() -> (Self, PlaybackTicket) {
        let state = Arc::new(PlaybackCompletionState {
            status: Mutex::new(None),
            changed: Condvar::new(),
        });
        (
            Self {
                state: state.clone(),
                reported: false,
            },
            PlaybackTicket { state },
        )
    }

    fn report(&mut self, status: PlaybackStatus) {
        if self.reported {
            return;
        }
        self.reported = true;
        *self.state.status.lock().unwrap() = Some(status);
        self.state.changed.notify_all();
    }
}

impl Drop for PlaybackCompletion {
    fn drop(&mut self) {
        self.report(PlaybackStatus::Cancelled);
    }
}

#[derive(Default)]
struct ScheduledPlaybackState {
    pending: Mutex<usize>,
    changed: Condvar,
}

impl ScheduledPlaybackState {
    fn begin(self: &Arc<Self>) -> ScheduledPlaybackGuard {
        *self.pending.lock().unwrap() += 1;
        ScheduledPlaybackGuard {
            state: self.clone(),
        }
    }

    fn wait(&self) {
        let mut pending = self.pending.lock().unwrap();
        while *pending > 0 {
            pending = self.changed.wait(pending).unwrap();
        }
    }
}

struct ScheduledPlaybackGuard {
    state: Arc<ScheduledPlaybackState>,
}

#[derive(Default)]
struct NullPendingState {
    count: Mutex<usize>,
    changed: Condvar,
}

impl NullPendingState {
    fn begin(&self) {
        *self.count.lock().unwrap() += 1;
    }

    fn finish(&self) {
        let mut count = self.count.lock().unwrap();
        *count = count.saturating_sub(1);
        self.changed.notify_all();
    }

    fn len(&self) -> usize {
        *self.count.lock().unwrap()
    }

    fn wait(&self) {
        let mut count = self.count.lock().unwrap();
        while *count > 0 {
            count = self.changed.wait(count).unwrap();
        }
    }
}

struct NullQueuedSource {
    generation: u64,
    source: Box<dyn Source<Item = f32> + Send>,
}

enum NullCommand {
    Append(NullQueuedSource),
    Shutdown,
}

struct NullSink {
    sender: Sender<NullCommand>,
    generation: Arc<AtomicU64>,
    pending: Arc<NullPendingState>,
}

impl NullSink {
    fn append<S>(&self, source: S)
    where
        S: Source<Item = f32> + Send + 'static,
    {
        self.pending.begin();
        let command = NullCommand::Append(NullQueuedSource {
            generation: self.generation.load(Ordering::Acquire),
            source: Box::new(source),
        });
        if self.sender.send(command).is_err() {
            self.pending.finish();
        }
    }

    fn clear(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.pending.wait();
    }

    fn shutdown(&self) {
        let _ = self.sender.send(NullCommand::Shutdown);
    }
}

#[derive(Clone)]
enum ManagedSink {
    Device(Arc<Sink>),
    Null(Arc<NullSink>),
}

impl ManagedSink {
    fn append<S>(&self, source: S)
    where
        S: Source<Item = f32> + Send + 'static,
    {
        match self {
            Self::Device(sink) => sink.append(source),
            Self::Null(sink) => sink.append(source),
        }
    }

    fn clear(&self) {
        match self {
            Self::Device(sink) => sink.clear(),
            Self::Null(sink) => sink.clear(),
        }
    }

    fn play(&self) {
        if let Self::Device(sink) = self {
            sink.play();
        }
    }

    fn empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        match self {
            Self::Device(sink) => sink.len(),
            Self::Null(sink) => sink.pending.len(),
        }
    }

    fn sleep_until_end(&self) {
        match self {
            Self::Device(sink) => sink.sleep_until_end(),
            Self::Null(sink) => sink.pending.wait(),
        }
    }

    fn shutdown(&self) {
        if let Self::Null(sink) = self {
            sink.shutdown();
        }
    }
}

fn run_null_sink(
    receiver: Receiver<NullCommand>,
    generation: Arc<AtomicU64>,
    pending: Arc<NullPendingState>,
    shutdown: Arc<AtomicBool>,
) {
    while let Ok(command) = receiver.recv() {
        let NullCommand::Append(mut queued) = command else {
            break;
        };
        loop {
            if shutdown.load(Ordering::Acquire)
                || generation.load(Ordering::Acquire) != queued.generation
            {
                break;
            }
            let consumed = queued.source.by_ref().take(NULL_AUDIO_POLL_SAMPLES).count();
            if consumed < NULL_AUDIO_POLL_SAMPLES {
                break;
            }
        }
        drop(queued);
        pending.finish();
    }

    while let Ok(command) = receiver.try_recv() {
        if matches!(command, NullCommand::Append(_)) {
            pending.finish();
        }
    }
}

impl Drop for ScheduledPlaybackGuard {
    fn drop(&mut self) {
        let mut pending = self.state.pending.lock().unwrap();
        *pending = pending.saturating_sub(1);
        self.state.changed.notify_all();
    }
}

/// Thread-safe handle to the three audio sinks.
///
/// Both the reader thread (for `stop_all` on `s` command) and the synthesis
/// worker thread (for queuing synthesized audio) hold an `Arc<AudioControl>`.
/// Both managed sink implementations are `Send + Sync`, so this type is too.
#[derive(Clone)]
pub struct AudioControl {
    speech_sink: ManagedSink,
    tone_sink: ManagedSink,
    sound_sink: ManagedSink,
    speech_max: usize,
    tone_max: usize,
    sound_max: usize,
    schedule_generations: Arc<[AtomicU64; 3]>,
    stream_gates: Arc<[Mutex<()>; 3]>,
    speech_stop_cancellation: Arc<Mutex<CancellationToken>>,
    tone_stop_cancellation: Arc<Mutex<CancellationToken>>,
    scheduled_playback: Arc<ScheduledPlaybackState>,
}

impl AudioControl {
    fn new(
        speech_sink: ManagedSink,
        tone_sink: ManagedSink,
        sound_sink: ManagedSink,
        speech_max: usize,
        tone_max: usize,
        sound_max: usize,
    ) -> Self {
        Self {
            speech_sink,
            tone_sink,
            sound_sink,
            speech_max,
            tone_max,
            sound_max,
            schedule_generations: Arc::new([
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ]),
            stream_gates: Arc::new([Mutex::new(()), Mutex::new(()), Mutex::new(())]),
            speech_stop_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            tone_stop_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            scheduled_playback: Arc::new(ScheduledPlaybackState::default()),
        }
    }

    fn sink_and_max(&self, stream: StreamType) -> (&ManagedSink, usize) {
        match stream {
            StreamType::Speech => (&self.speech_sink, self.speech_max),
            StreamType::Tone => (&self.tone_sink, self.tone_max),
            StreamType::Sound => (&self.sound_sink, self.sound_max),
        }
    }

    fn smooth_stop_cancellation(&self, stream: StreamType) -> Option<CancellationToken> {
        match stream {
            StreamType::Speech => Some(self.speech_stop_cancellation.lock().unwrap().clone()),
            StreamType::Tone => Some(self.tone_stop_cancellation.lock().unwrap().clone()),
            StreamType::Sound => None,
        }
    }

    fn smooth_stop_fade_frames(stream: StreamType) -> Option<usize> {
        match stream {
            StreamType::Speech => Some(SPEECH_STOP_FADE_FRAMES),
            StreamType::Tone => Some(TONE_STOP_FADE_FRAMES),
            StreamType::Sound => None,
        }
    }

    /// Queue an audio buffer on the given stream.
    ///
    /// If the stream's backlog is at capacity, clears old items first.
    /// Returns `true` if audio was queued, `false` if the buffer was empty.
    pub fn queue(&self, stream: StreamType, buffer: &AudioBuffer) -> Result<bool, AudioError> {
        self.queue_if(stream, buffer, || true)
    }

    /// Queue audio only while PREDICATE remains true at the stop/queue gate.
    ///
    /// Stop holds the same per-stream gate. This makes a generation check and
    /// append atomic with respect to clearing playback: stale PCM can never be
    /// appended immediately after a newer stop has cleared the sink.
    pub fn queue_if<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        predicate: F,
    ) -> Result<bool, AudioError>
    where
        F: FnOnce() -> bool,
    {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(false);
        }
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(false);
        };

        let samples = buffer.samples.clone();
        if let Some(cancellation) = self.smooth_stop_cancellation(stream) {
            sink.append(SmoothStopBufferSource::new(
                samples,
                cancellation,
                Self::smooth_stop_fade_frames(stream)
                    .expect("a smooth-stop stream has a fade duration"),
            ));
        } else {
            sink.append(BufferSource::new(samples));
        }
        sink.play();
        Ok(true)
    }

    /// Queue an audio buffer and return a ticket reporting its terminal state.
    ///
    /// Natural source exhaustion reports [`PlaybackStatus::Completed`]. Stop,
    /// backlog clearing, or sink teardown reports [`PlaybackStatus::Cancelled`].
    /// An empty buffer returns `None` because it has no playback lifetime.
    pub fn queue_tracked(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
    ) -> Result<Option<PlaybackTicket>, AudioError> {
        self.queue_tracked_if(stream, buffer, || true)
    }

    /// Queue tracked audio only while PREDICATE is true at the stop/queue gate.
    pub fn queue_tracked_if<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        self.queue_tracked_if_with_cancellation(stream, buffer, None, predicate)
    }

    /// Queue tracked audio with cooperative cancellation while PREDICATE is
    /// true at the stop/queue gate.
    ///
    /// Cancelling `cancellation` ends this source without clearing unrelated
    /// sources from the same stream. The returned ticket reports cancellation.
    pub fn queue_tracked_cancellable_if<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cancellation: CancellationToken,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        self.queue_tracked_if_with_cancellation(stream, buffer, Some(cancellation), predicate)
    }

    fn queue_tracked_if_with_cancellation<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cancellation: Option<CancellationToken>,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(None);
        }
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(None);
        };

        let samples = buffer.samples.clone();
        let (source, ticket) = TrackedBufferSource::new_with_options(
            samples,
            cancellation,
            self.smooth_stop_cancellation(stream),
            Self::smooth_stop_fade_frames(stream),
        );
        sink.append(source);
        sink.play();
        Ok(Some(ticket))
    }

    /// Queue an overlay after every primary playback BARRIER completes.
    ///
    /// The returned ticket exists immediately and covers the wait plus the
    /// overlay's full audible tail. If a barrier or a subsequent sound-stream
    /// stop is cancelled, the overlay is never queued and the ticket reports
    /// cancellation. Scheduling never blocks the synthesis worker.
    pub fn queue_overlay_after(
        &self,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
    ) -> Result<Option<PlaybackTicket>, AudioError> {
        self.queue_overlay_after_if(buffer, barriers, || true)
    }

    /// Queue an overlay after its barriers only while PREDICATE is current.
    pub fn queue_overlay_after_if<F>(
        &self,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        self.queue_stream_after_if_with_cancellation(
            StreamType::Sound,
            buffer,
            barriers,
            None,
            predicate,
        )
    }

    /// Queue an overlay after its barriers with cooperative cancellation.
    pub fn queue_overlay_after_cancellable_if<F>(
        &self,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
        cancellation: CancellationToken,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        self.queue_stream_after_if_with_cancellation(
            StreamType::Sound,
            buffer,
            barriers,
            Some(cancellation),
            predicate,
        )
    }

    /// Queue one stream after every playback barrier completes.
    ///
    /// This is the boundary-level scheduling primitive used by overlay tracks.
    /// The scheduled stream retains its own volume/routing pipeline and does
    /// not advance the speech sink. Stopping that stream cancels work which has
    /// not reached its boundary yet.
    pub fn queue_stream_after(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
    ) -> Result<Option<PlaybackTicket>, AudioError> {
        self.queue_stream_after_if(stream, buffer, barriers, || true)
    }

    /// Queue one stream after its barriers only while PREDICATE is current.
    pub fn queue_stream_after_if<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        self.queue_stream_after_if_with_cancellation(stream, buffer, barriers, None, predicate)
    }

    fn queue_stream_after_if_with_cancellation<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        barriers: Vec<PlaybackTicket>,
        cancellation: Option<CancellationToken>,
        predicate: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnOnce() -> bool,
    {
        if buffer.is_empty() {
            return Ok(None);
        }
        if barriers.is_empty() {
            return self.queue_tracked_if_with_cancellation(
                stream,
                buffer,
                cancellation,
                predicate,
            );
        }

        let stream_index = stream_index(stream);
        let generation = {
            let _gate = self.stream_gates[stream_index].lock().unwrap();
            if !predicate() {
                return Ok(None);
            }
            self.schedule_generations[stream_index].load(Ordering::Acquire)
        };
        let control = self.clone();
        let samples = buffer.samples.clone();
        let (mut completion, ticket) = PlaybackCompletion::pair();
        let guard = self.scheduled_playback.begin();
        std::thread::Builder::new()
            .name("omnivox-overlay-scheduler".to_owned())
            .spawn(move || {
                let _guard = guard;
                if barriers
                    .into_iter()
                    .any(|barrier| barrier.wait() == PlaybackStatus::Cancelled)
                    || cancellation
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled)
                    || control.schedule_generations[stream_index].load(Ordering::Acquire)
                        != generation
                {
                    completion.report(PlaybackStatus::Cancelled);
                    return;
                }
                let overlay = AudioBuffer::new(samples);
                let status = match control.queue_tracked_if_with_cancellation(
                    stream,
                    &overlay,
                    cancellation,
                    || {
                        control.schedule_generations[stream_index].load(Ordering::Acquire)
                            == generation
                    },
                ) {
                    Ok(Some(actual)) => actual.wait(),
                    Ok(None) => PlaybackStatus::Completed,
                    Err(error) => {
                        tracing::warn!("scheduled overlay queue error: {}", error);
                        PlaybackStatus::Cancelled
                    }
                };
                completion.report(status);
            })
            .map_err(|error| {
                AudioError::PlaybackError(format!("overlay scheduler thread: {error}"))
            })?;
        Ok(Some(ticket))
    }

    /// Queue tracked audio with caller-defined frame cues.
    ///
    /// Cues are emitted through `cue_sender` as the source reaches their frame
    /// boundaries. They are ordered by frame offset; cues at the same frame
    /// preserve their input order. An offset equal to the buffer frame count
    /// is emitted immediately before natural completion. Offsets beyond the
    /// buffer are rejected. Cancellation drops all cues not yet reached. Cue
    /// timing follows source consumption and may lead acoustic output by the
    /// audio device's buffering latency.
    pub fn queue_tracked_with_cues(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cues: Vec<PlaybackCue>,
        cue_sender: Sender<PlaybackCue>,
    ) -> Result<Option<PlaybackTicket>, AudioError> {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        let cues = prepare_playback_cues(cues, buffer.frame_count())?;
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(None);
        };

        let (source, ticket) = TrackedBufferSource::new_with_cue_options(
            buffer.samples.clone(),
            cues,
            Some(cue_sender),
            None,
            None,
            self.smooth_stop_cancellation(stream),
            Self::smooth_stop_fade_frames(stream),
        );
        sink.append(source);
        sink.play();
        Ok(Some(ticket))
    }

    /// Queue tracked audio and invoke `on_cue` at caller-defined frame cues.
    ///
    /// This has the same ordering, bounds, cancellation, and source-consumption
    /// timing contract as [`Self::queue_tracked_with_cues`]. The callback runs
    /// on the audio source thread and must return quickly without blocking.
    pub fn queue_tracked_with_cue_callback<F>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cues: Vec<PlaybackCue>,
        on_cue: F,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnMut(PlaybackCue) + Send + 'static,
    {
        self.queue_tracked_with_cue_callback_if(stream, buffer, cues, on_cue, || true)
    }

    /// Queue callback-tracked audio only while PREDICATE is true at the
    /// stop/queue gate.
    pub fn queue_tracked_with_cue_callback_if<F, P>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cues: Vec<PlaybackCue>,
        on_cue: F,
        predicate: P,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnMut(PlaybackCue) + Send + 'static,
        P: FnOnce() -> bool,
    {
        self.queue_tracked_with_cue_callback_if_with_cancellation(
            stream, buffer, cues, on_cue, None, predicate,
        )
    }

    /// Queue callback-tracked audio with cooperative cancellation while
    /// PREDICATE is true at the stop/queue gate.
    pub fn queue_tracked_with_cue_callback_cancellable_if<F, P>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cues: Vec<PlaybackCue>,
        on_cue: F,
        cancellation: CancellationToken,
        predicate: P,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnMut(PlaybackCue) + Send + 'static,
        P: FnOnce() -> bool,
    {
        self.queue_tracked_with_cue_callback_if_with_cancellation(
            stream,
            buffer,
            cues,
            on_cue,
            Some(cancellation),
            predicate,
        )
    }

    /// Queue one bounded progressive speech source with dynamic frame cues.
    ///
    /// Device playback is appended after three PCM windows have been supplied,
    /// or when a shorter stream completes. Null playback is appended
    /// immediately because its worker is not a real-time device. The source
    /// remains one speech-queue item while the returned producer supplies
    /// subsequent canonical PCM windows. Dropping the producer without calling
    /// [`ProgressivePlaybackProducer::finish`] cancels the playback ticket.
    pub fn queue_progressive_speech_with_cue_callback_cancellable_if<F, P>(
        &self,
        on_cue: F,
        cancellation: CancellationToken,
        predicate: P,
    ) -> Result<Option<(ProgressivePlaybackProducer, PlaybackTicket)>, AudioError>
    where
        F: FnMut(PlaybackCue) + Send + 'static,
        P: FnOnce() -> bool,
    {
        let stream = StreamType::Speech;
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(None);
        }
        let (sink, max_depth) = self.sink_and_max(stream);
        if sink.len() >= max_depth {
            debug!(
                "Stream {:?} at capacity ({}/{}), clearing backlog",
                stream,
                sink.len(),
                max_depth
            );
            let previous = std::mem::replace(
                &mut *self.speech_stop_cancellation.lock().unwrap(),
                CancellationToken::new(),
            );
            previous.cancel();
            sink.clear();
            sink.play();
        }

        let stream_cancellation = self
            .smooth_stop_cancellation(stream)
            .expect("speech has a stream cancellation token");
        let (mut producer, source, ticket) = ProgressivePlaybackSource::new(
            Box::new(on_cue),
            cancellation,
            stream_cancellation,
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        if matches!(sink, ManagedSink::Device(_)) {
            producer.pending_attachment = Some(ProgressivePlaybackAttachment {
                control: self.clone(),
                source: Some(source),
                generation: self.schedule_generations[stream_index(stream)].load(Ordering::Acquire),
            });
        } else {
            sink.append(source);
            sink.play();
        }
        Ok(Some((producer, ticket)))
    }

    fn queue_tracked_with_cue_callback_if_with_cancellation<F, P>(
        &self,
        stream: StreamType,
        buffer: &AudioBuffer,
        cues: Vec<PlaybackCue>,
        on_cue: F,
        cancellation: Option<CancellationToken>,
        predicate: P,
    ) -> Result<Option<PlaybackTicket>, AudioError>
    where
        F: FnMut(PlaybackCue) + Send + 'static,
        P: FnOnce() -> bool,
    {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(None);
        }
        let cues = prepare_playback_cues(cues, buffer.frame_count())?;
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(None);
        };

        let samples = buffer.samples.clone();
        let (source, ticket) = TrackedBufferSource::new_with_cue_options(
            samples,
            cues,
            None,
            Some(Box::new(on_cue)),
            cancellation,
            self.smooth_stop_cancellation(stream),
            Self::smooth_stop_fade_frames(stream),
        );
        sink.append(source);
        sink.play();
        Ok(Some(ticket))
    }

    fn prepare_queue(&self, stream: StreamType, buffer: &AudioBuffer) -> Option<&ManagedSink> {
        if buffer.is_empty() {
            return None;
        }

        let (sink, max_depth) = self.sink_and_max(stream);
        if sink.len() >= max_depth {
            debug!(
                "Stream {:?} at capacity ({}/{}), clearing backlog",
                stream,
                sink.len(),
                max_depth
            );
            sink.clear();
            sink.play();
        }
        Some(sink)
    }

    /// Stop a specific stream, retiring all queued and playing audio.
    ///
    /// Speech and tones already being consumed use short smooth fades to avoid
    /// waveform discontinuities. Sources which have not started, plus sound
    /// streams, still stop immediately.
    pub fn stop(&self, stream: StreamType) {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        self.schedule_generations[stream_index(stream)].fetch_add(1, Ordering::AcqRel);
        let (sink, _) = self.sink_and_max(stream);
        let smooth_stop = match stream {
            StreamType::Speech => Some(&self.speech_stop_cancellation),
            StreamType::Tone => Some(&self.tone_stop_cancellation),
            StreamType::Sound => None,
        };
        if let Some(cancellation) = smooth_stop {
            let previous =
                std::mem::replace(&mut *cancellation.lock().unwrap(), CancellationToken::new());
            previous.cancel();
        } else {
            sink.clear();
        }
        sink.play();
    }

    /// Stop every stream, using a short de-click fade for active speech and tones.
    pub fn stop_all(&self) {
        self.stop(StreamType::Speech);
        self.stop(StreamType::Tone);
        self.stop(StreamType::Sound);
    }

    /// Check if a stream is currently playing audio.
    pub fn is_playing(&self, stream: StreamType) -> bool {
        let (sink, _) = self.sink_and_max(stream);
        !sink.empty()
    }

    /// Get the number of items pending on a stream (including currently playing).
    pub fn pending(&self, stream: StreamType) -> usize {
        let (sink, _) = self.sink_and_max(stream);
        sink.len()
    }

    /// Block until all three streams have finished playing.
    ///
    /// Call this after all synthesis is done (worker thread joined) to ensure
    /// all queued audio plays out before the `OutputStream` is dropped.
    pub fn drain(&self) {
        self.scheduled_playback.wait();
        self.speech_sink.sleep_until_end();
        self.tone_sink.sleep_until_end();
        self.sound_sink.sleep_until_end();
    }
}

/// Concurrent audio streams with per-stream serialization and backlog limits.
///
/// Owns the selected output runtime and delegates all audio operations to an
/// inner `Arc<AudioControl>`. Call `control()` to get a shareable handle for
/// use from other threads (e.g. synthesis worker thread).
pub struct AudioStreams {
    runtime: AudioStreamRuntime,
    control: Arc<AudioControl>,
}

enum AudioStreamRuntime {
    Device {
        _stream: OutputStream,
        _stream_handle: OutputStreamHandle,
    },
    Null {
        shutdown: Arc<AtomicBool>,
        workers: Vec<Option<JoinHandle<()>>>,
    },
}

impl AudioStreams {
    /// Create three audio streams on the default output device.
    ///
    /// Each stream has an independent backlog limit. When the limit is reached,
    /// the stream is cleared and only the new item plays (stay current, don't
    /// play catch-up).
    pub fn new(
        speech_max_depth: usize,
        tone_max_depth: usize,
        sound_max_depth: usize,
    ) -> Result<Self, AudioError> {
        Self::new_with_backend(
            speech_max_depth,
            tone_max_depth,
            sound_max_depth,
            AudioBackend::Device,
        )
    }

    /// Create three streams using the selected output backend.
    pub fn new_with_backend(
        speech_max_depth: usize,
        tone_max_depth: usize,
        sound_max_depth: usize,
        backend: AudioBackend,
    ) -> Result<Self, AudioError> {
        match backend {
            AudioBackend::Device => {
                Self::new_device(speech_max_depth, tone_max_depth, sound_max_depth)
            }
            AudioBackend::Null => Self::new_null(speech_max_depth, tone_max_depth, sound_max_depth),
        }
    }

    fn new_device(
        speech_max_depth: usize,
        tone_max_depth: usize,
        sound_max_depth: usize,
    ) -> Result<Self, AudioError> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| AudioError::DeviceNotFound(format!("default device: {}", e)))?;

        let speech_sink = ManagedSink::Device(Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("speech sink: {}", e)))?,
        ));
        let tone_sink = ManagedSink::Device(Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("tone sink: {}", e)))?,
        ));
        let sound_sink = ManagedSink::Device(Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("sound sink: {}", e)))?,
        ));

        let control = Arc::new(AudioControl::new(
            speech_sink,
            tone_sink,
            sound_sink,
            speech_max_depth,
            tone_max_depth,
            sound_max_depth,
        ));

        Ok(Self {
            runtime: AudioStreamRuntime::Device {
                _stream: stream,
                _stream_handle: stream_handle,
            },
            control,
        })
    }

    fn new_null(
        speech_max_depth: usize,
        tone_max_depth: usize,
        sound_max_depth: usize,
    ) -> Result<Self, AudioError> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut sinks = Vec::with_capacity(3);
        let mut workers = Vec::with_capacity(3);
        for name in ["speech", "tone", "sound"] {
            let (sender, receiver) = mpsc::channel();
            let generation = Arc::new(AtomicU64::new(0));
            let pending = Arc::new(NullPendingState::default());
            let worker_generation = generation.clone();
            let worker_pending = pending.clone();
            let worker_shutdown = shutdown.clone();
            let worker = std::thread::Builder::new()
                .name(format!("omnivox-null-{name}"))
                .spawn(move || {
                    run_null_sink(receiver, worker_generation, worker_pending, worker_shutdown)
                })
                .map_err(|error| {
                    AudioError::PlaybackError(format!("null {name} worker: {error}"))
                })?;
            sinks.push(ManagedSink::Null(Arc::new(NullSink {
                sender,
                generation,
                pending,
            })));
            workers.push(Some(worker));
        }
        let [speech_sink, tone_sink, sound_sink]: [ManagedSink; 3] = sinks
            .try_into()
            .map_err(|_| AudioError::PlaybackError("null sink construction failed".to_owned()))?;
        let control = Arc::new(AudioControl::new(
            speech_sink,
            tone_sink,
            sound_sink,
            speech_max_depth,
            tone_max_depth,
            sound_max_depth,
        ));

        Ok(Self {
            runtime: AudioStreamRuntime::Null { shutdown, workers },
            control,
        })
    }

    /// Get a thread-safe handle to the audio controls.
    ///
    /// The returned `Arc<AudioControl>` is `Send + Sync` and can be cloned and
    /// sent to the synthesis worker thread. `AudioStreams` must remain alive so
    /// its device stream or null-consumer workers can process queued sources.
    pub fn control(&self) -> Arc<AudioControl> {
        self.control.clone()
    }

    /// Queue an audio buffer on the given stream.
    pub fn queue(&self, stream: StreamType, buffer: &AudioBuffer) -> Result<bool, AudioError> {
        self.control.queue(stream, buffer)
    }

    /// Stop a specific stream, smoothing active speech or tones over a short fade.
    pub fn stop(&self, stream: StreamType) {
        self.control.stop(stream)
    }

    /// Stop every stream, using the short de-click fade for active speech and tones.
    pub fn stop_all(&self) {
        self.control.stop_all()
    }

    /// Check if a stream is currently playing audio.
    pub fn is_playing(&self, stream: StreamType) -> bool {
        self.control.is_playing(stream)
    }

    /// Get the number of items pending on a stream (including currently playing).
    pub fn pending(&self, stream: StreamType) -> usize {
        self.control.pending(stream)
    }

    /// Block until all streams have finished playing.
    pub fn drain(&self) {
        self.control.drain();
    }
}

impl Drop for AudioStreams {
    fn drop(&mut self) {
        let AudioStreamRuntime::Null { shutdown, workers } = &mut self.runtime else {
            return;
        };
        self.control
            .speech_stop_cancellation
            .lock()
            .unwrap()
            .cancel();
        self.control.tone_stop_cancellation.lock().unwrap().cancel();
        shutdown.store(true, Ordering::Release);
        self.control.speech_sink.shutdown();
        self.control.tone_sink.shutdown();
        self.control.sound_sink.shutdown();
        for worker in workers {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

/// Audio output device for single-shot playback via rodio.
pub struct AudioOutput {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
}

impl AudioOutput {
    /// Create a new AudioOutput using the default output device.
    pub fn new() -> Result<Self, AudioError> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| AudioError::DeviceNotFound(format!("default device: {}", e)))?;

        Ok(Self {
            _stream: stream,
            stream_handle,
        })
    }

    /// Play an AudioBuffer to completion (blocking).
    pub fn play_blocking(&self, buffer: &AudioBuffer) -> Result<(), AudioError> {
        if buffer.is_empty() {
            return Ok(());
        }

        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| AudioError::PlaybackError(format!("failed to create sink: {}", e)))?;

        let source = BufferSource::new(buffer.samples.clone());
        sink.append(source);
        sink.sleep_until_end();

        Ok(())
    }

    /// Play an AudioBuffer without blocking. Returns a PlaybackHandle
    /// that can be used to stop playback.
    pub fn play(&self, buffer: &AudioBuffer) -> Result<PlaybackHandle, AudioError> {
        if buffer.is_empty() {
            return Ok(PlaybackHandle {
                sink: Arc::new(Sink::try_new(&self.stream_handle).map_err(|e| {
                    AudioError::PlaybackError(format!("failed to create sink: {}", e))
                })?),
            });
        }

        let sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| AudioError::PlaybackError(format!("failed to create sink: {}", e)))?;

        let source = BufferSource::new(buffer.samples.clone());
        sink.append(source);

        Ok(PlaybackHandle {
            sink: Arc::new(sink),
        })
    }
}

/// Handle to a playing audio buffer. Can be used to stop playback.
pub struct PlaybackHandle {
    sink: Arc<Sink>,
}

impl PlaybackHandle {
    /// Stop playback immediately.
    pub fn stop(&self) {
        self.sink.stop();
    }

    /// Check if playback has finished.
    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    /// Block until playback completes.
    pub fn wait(&self) {
        self.sink.sleep_until_end();
    }
}

/// A rodio Source backed by a Vec of interleaved stereo f32 samples.
struct BufferSource {
    samples: Vec<f32>,
    position: usize,
}

impl BufferSource {
    fn new(samples: Vec<f32>) -> Self {
        Self {
            samples,
            position: 0,
        }
    }
}

impl Iterator for BufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.position < self.samples.len() {
            let sample = self.samples[self.position];
            self.position += 1;
            Some(sample)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.samples.len() - self.position;
        (remaining, Some(remaining))
    }
}

impl Source for BufferSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.position)
    }

    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        let frames = self.samples.len() / CHANNELS as usize;
        Some(Duration::from_secs_f64(frames as f64 / SAMPLE_RATE as f64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancellationFadeState {
    Playing,
    Fading { samples_emitted: usize },
    Finished,
}

fn smooth_fade_gain(fade_frames: usize, frame: usize) -> f32 {
    if fade_frames <= 1 {
        return 0.0;
    }
    let remaining = (fade_frames - 1 - frame) as f32 / (fade_frames - 1) as f32;
    remaining * remaining * (3.0 - 2.0 * remaining)
}

/// Per-source cancellation state which can de-click an active speech source.
///
/// A source cancelled before its first sample is discarded immediately. Once
/// a source has started, cancellation begins only at a complete interleaved
/// frame boundary and applies one gain value to every channel in that frame.
struct PlaybackCancellation {
    request: Option<CancellationToken>,
    stream: Option<CancellationToken>,
    fade_frames: Option<usize>,
    state: CancellationFadeState,
}

impl PlaybackCancellation {
    fn new(
        request: Option<CancellationToken>,
        stream: Option<CancellationToken>,
        fade_frames: Option<usize>,
    ) -> Self {
        Self {
            request,
            stream,
            fade_frames,
            state: CancellationFadeState::Playing,
        }
    }

    fn is_requested(&self) -> bool {
        self.request
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
            || self
                .stream
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    fn has_begun(&self) -> bool {
        self.state != CancellationFadeState::Playing
    }

    fn fade_samples(&self) -> usize {
        self.fade_frames.unwrap_or(0) * CHANNELS as usize
    }

    /// Return the gain for the next sample, or `None` once cancellation is
    /// terminal. POSITION is the next interleaved sample in the inner source.
    fn next_gain(&mut self, position: usize) -> Option<f32> {
        if self.state == CancellationFadeState::Finished {
            return None;
        }

        if self.state == CancellationFadeState::Playing
            && position.is_multiple_of(CHANNELS as usize)
            && self.is_requested()
        {
            let fade_samples = self.fade_samples();
            if position == 0 || fade_samples == 0 {
                self.state = CancellationFadeState::Finished;
                return None;
            }
            self.state = CancellationFadeState::Fading { samples_emitted: 0 };
        }

        let CancellationFadeState::Fading { samples_emitted } = &mut self.state else {
            return Some(1.0);
        };
        let frame = *samples_emitted / CHANNELS as usize;
        let fade_frames = self.fade_frames.unwrap_or(0);
        let gain = smooth_fade_gain(fade_frames, frame);
        *samples_emitted += 1;
        if *samples_emitted >= fade_frames * CHANNELS as usize {
            self.state = CancellationFadeState::Finished;
        }
        Some(gain)
    }

    fn remaining_samples(&self, position: usize) -> usize {
        match self.state {
            CancellationFadeState::Finished => 0,
            CancellationFadeState::Fading { samples_emitted } => {
                self.fade_samples().saturating_sub(samples_emitted)
            }
            CancellationFadeState::Playing if self.is_requested() => {
                let fade_samples = self.fade_samples();
                if position == 0 || fade_samples == 0 {
                    0
                } else {
                    let channels = CHANNELS as usize;
                    let frame_remainder = position % channels;
                    let samples_to_boundary = if frame_remainder == 0 {
                        0
                    } else {
                        channels - frame_remainder
                    };
                    samples_to_boundary + fade_samples
                }
            }
            CancellationFadeState::Playing => usize::MAX,
        }
    }
}

/// Untracked audio which participates in stream-wide smooth stopping.
struct SmoothStopBufferSource {
    inner: BufferSource,
    cancellation: PlaybackCancellation,
}

impl SmoothStopBufferSource {
    fn new(samples: Vec<f32>, cancellation: CancellationToken, fade_frames: usize) -> Self {
        Self {
            inner: BufferSource::new(samples),
            cancellation: PlaybackCancellation::new(None, Some(cancellation), Some(fade_frames)),
        }
    }
}

impl Iterator for SmoothStopBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let gain = self.cancellation.next_gain(self.inner.position)?;
        self.inner.next().map(|sample| sample * gain)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let limit = self.cancellation.remaining_samples(self.inner.position);
        let (_, maximum) = self.inner.size_hint();
        (0, maximum.map(|maximum| maximum.min(limit)))
    }
}

impl Source for SmoothStopBufferSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len().map(|remaining| {
            remaining.min(self.cancellation.remaining_samples(self.inner.position))
        })
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

/// Buffer source that reports natural exhaustion and cancellation distinctly.
struct TrackedBufferSource {
    inner: BufferSource,
    completion: PlaybackCompletion,
    cancellation: PlaybackCancellation,
    cues: Vec<PlaybackCue>,
    next_cue: usize,
    cue_sender: Option<Sender<PlaybackCue>>,
    cue_callback: Option<Box<dyn FnMut(PlaybackCue) + Send>>,
}

impl TrackedBufferSource {
    #[cfg(test)]
    fn new(samples: Vec<f32>) -> (Self, PlaybackTicket) {
        Self::new_with_options(samples, None, None, None)
    }

    #[cfg(test)]
    fn new_cancellable(
        samples: Vec<f32>,
        cancellation: CancellationToken,
    ) -> (Self, PlaybackTicket) {
        Self::new_with_options(
            samples,
            Some(cancellation),
            None,
            Some(SPEECH_STOP_FADE_FRAMES),
        )
    }

    fn new_with_options(
        samples: Vec<f32>,
        request_cancellation: Option<CancellationToken>,
        stream_cancellation: Option<CancellationToken>,
        fade_frames: Option<usize>,
    ) -> (Self, PlaybackTicket) {
        Self::new_with_cue_options(
            samples,
            Vec::new(),
            None,
            None,
            request_cancellation,
            stream_cancellation,
            fade_frames,
        )
    }

    #[cfg(test)]
    fn new_with_cues(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        cue_sender: Sender<PlaybackCue>,
    ) -> (Self, PlaybackTicket) {
        Self::new_with_cue_options(samples, cues, Some(cue_sender), None, None, None, None)
    }

    #[cfg(test)]
    fn new_with_cue_callback<F>(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        on_cue: F,
    ) -> (Self, PlaybackTicket)
    where
        F: FnMut(PlaybackCue) + Send + 'static,
    {
        Self::new_with_cue_options(
            samples,
            cues,
            None,
            Some(Box::new(on_cue)),
            None,
            None,
            None,
        )
    }

    #[cfg(test)]
    fn new_with_cue_callback_cancellable<F>(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        on_cue: F,
        cancellation: CancellationToken,
    ) -> (Self, PlaybackTicket)
    where
        F: FnMut(PlaybackCue) + Send + 'static,
    {
        Self::new_with_cue_options(
            samples,
            cues,
            None,
            Some(Box::new(on_cue)),
            Some(cancellation),
            None,
            Some(SPEECH_STOP_FADE_FRAMES),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_cue_options(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        cue_sender: Option<Sender<PlaybackCue>>,
        cue_callback: Option<Box<dyn FnMut(PlaybackCue) + Send>>,
        request_cancellation: Option<CancellationToken>,
        stream_cancellation: Option<CancellationToken>,
        fade_frames: Option<usize>,
    ) -> (Self, PlaybackTicket) {
        let (completion, ticket) = PlaybackCompletion::pair();
        (
            Self {
                inner: BufferSource::new(samples),
                completion,
                cancellation: PlaybackCancellation::new(
                    request_cancellation,
                    stream_cancellation,
                    fade_frames,
                ),
                cues,
                next_cue: 0,
                cue_sender,
                cue_callback,
            },
            ticket,
        )
    }

    fn report_cues_at_current_frame(&mut self) {
        if self.next_cue >= self.cues.len()
            || !self.inner.position.is_multiple_of(CHANNELS as usize)
        {
            return;
        }

        let current_frame = (self.inner.position / CHANNELS as usize) as u64;
        while self.next_cue < self.cues.len()
            && self.cues[self.next_cue].frame_offset == current_frame
        {
            let cue = self.cues[self.next_cue];
            self.next_cue += 1;
            if let Some(callback) = &mut self.cue_callback {
                callback(cue);
                continue;
            }
            let delivered = self
                .cue_sender
                .as_ref()
                .is_some_and(|sender| sender.send(cue).is_ok());
            if !delivered {
                self.cue_sender = None;
                self.next_cue = self.cues.len();
                break;
            }
        }
    }

    fn report(&mut self, status: PlaybackStatus) {
        self.completion.report(status);
    }
}

impl Iterator for TrackedBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(gain) = self.cancellation.next_gain(self.inner.position) else {
            self.report(PlaybackStatus::Cancelled);
            return None;
        };
        if !self.cancellation.has_begun() {
            self.report_cues_at_current_frame();
        }
        let sample = self.inner.next().map(|sample| sample * gain);
        if sample.is_none() {
            let status = if self.cancellation.has_begun() || self.cancellation.is_requested() {
                PlaybackStatus::Cancelled
            } else {
                PlaybackStatus::Completed
            };
            self.report(status);
        }
        sample
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let limit = self.cancellation.remaining_samples(self.inner.position);
        let (_, maximum) = self.inner.size_hint();
        (0, maximum.map(|maximum| maximum.min(limit)))
    }
}

fn prepare_playback_cues(
    mut cues: Vec<PlaybackCue>,
    frame_count: usize,
) -> Result<Vec<PlaybackCue>, AudioError> {
    let frame_count = frame_count as u64;
    if let Some(cue) = cues.iter().find(|cue| cue.frame_offset > frame_count) {
        return Err(AudioError::InvalidFormat(format!(
            "playback cue {} offset {} exceeds buffer frame count {}",
            cue.identifier, cue.frame_offset, frame_count
        )));
    }
    cues.sort_by_key(|cue| cue.frame_offset);
    Ok(cues)
}

impl Source for TrackedBufferSource {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len().map(|remaining| {
            remaining.min(self.cancellation.remaining_samples(self.inner.position))
        })
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

impl Drop for TrackedBufferSource {
    fn drop(&mut self) {
        self.report(PlaybackStatus::Cancelled);
    }
}

/// Tracked rodio source that stays alive while bounded PCM windows arrive.
struct ProgressivePlaybackSource {
    receiver: Receiver<ProgressivePlaybackMessage>,
    current: BufferSource,
    position: usize,
    completion: PlaybackCompletion,
    cancellation: PlaybackCancellation,
    cues: VecDeque<PlaybackCue>,
    cue_callback: Box<dyn FnMut(PlaybackCue) + Send>,
}

impl ProgressivePlaybackSource {
    fn new(
        cue_callback: Box<dyn FnMut(PlaybackCue) + Send>,
        request_cancellation: CancellationToken,
        stream_cancellation: CancellationToken,
        fade_frames: Option<usize>,
    ) -> (ProgressivePlaybackProducer, Self, PlaybackTicket) {
        let (sender, receiver) = mpsc::sync_channel(PROGRESSIVE_PLAYBACK_CAPACITY);
        let (completion, ticket) = PlaybackCompletion::pair();
        let producer = ProgressivePlaybackProducer {
            sender: Some(sender),
            request_cancellation: request_cancellation.clone(),
            stream_cancellation: stream_cancellation.clone(),
            published_frames: 0,
            last_cue_offset: None,
            pending_cues: Vec::new(),
            prebuffered_audio_windows: 0,
            pending_attachment: None,
        };
        let source = Self {
            receiver,
            current: BufferSource::new(Vec::new()),
            position: 0,
            completion,
            cancellation: PlaybackCancellation::new(
                Some(request_cancellation),
                Some(stream_cancellation),
                fade_frames,
            ),
            cues: VecDeque::new(),
            cue_callback,
        };
        (producer, source, ticket)
    }

    fn report_cues_at_current_frame(&mut self) {
        if !self.position.is_multiple_of(CHANNELS as usize) {
            return;
        }
        let current_frame = (self.position / CHANNELS as usize) as u64;
        while self
            .cues
            .front()
            .is_some_and(|cue| cue.frame_offset == current_frame)
        {
            (self.cue_callback)(self.cues.pop_front().unwrap());
        }
    }

    fn report(&mut self, status: PlaybackStatus) {
        self.completion.report(status);
    }
}

impl Iterator for ProgressivePlaybackSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let Some(gain) = self.cancellation.next_gain(self.position) else {
                self.report(PlaybackStatus::Cancelled);
                return None;
            };
            if !self.cancellation.has_begun() {
                self.report_cues_at_current_frame();
            }
            if let Some(sample) = self.current.next() {
                self.position += 1;
                return Some(sample * gain);
            }
            if self.cancellation.has_begun() {
                self.report(PlaybackStatus::Cancelled);
                return None;
            }

            match self
                .receiver
                .recv_timeout(PROGRESSIVE_PLAYBACK_POLL_INTERVAL)
            {
                Ok(ProgressivePlaybackMessage::Audio { samples, cues }) => {
                    self.cues.extend(cues);
                    self.current = BufferSource::new(samples);
                }
                Ok(ProgressivePlaybackMessage::Complete { cues }) => {
                    self.cues.extend(cues);
                    self.report_cues_at_current_frame();
                    let status = if self.cues.is_empty() {
                        PlaybackStatus::Completed
                    } else {
                        PlaybackStatus::Cancelled
                    };
                    self.report(status);
                    return None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.report(PlaybackStatus::Cancelled);
                    return None;
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl Source for ProgressivePlaybackSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Drop for ProgressivePlaybackSource {
    fn drop(&mut self) {
        self.report(PlaybackStatus::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn cue(frame_offset: u64, identifier: u64) -> PlaybackCue {
        PlaybackCue {
            frame_offset,
            identifier,
        }
    }

    #[test]
    fn tracked_source_reports_natural_completion() {
        let (mut source, ticket) = TrackedBufferSource::new(vec![0.1, -0.1]);

        assert_eq!(source.next(), Some(0.1));
        assert_eq!(source.next(), Some(-0.1));
        assert_eq!(source.next(), None);
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn progressive_source_consumes_ordered_windows_and_dynamic_cues() {
        let request_cancellation = CancellationToken::new();
        let stream_cancellation = CancellationToken::new();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let (mut producer, mut source, ticket) = ProgressivePlaybackSource::new(
            Box::new(move |cue| cue_sender.send(cue).unwrap()),
            request_cancellation,
            stream_cancellation,
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        producer.push_cues(vec![cue(0, 10), cue(2, 20)]).unwrap();
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1]))
            .unwrap();
        producer
            .push_audio(AudioBuffer::new(vec![0.2, -0.2]))
            .unwrap();
        assert_eq!(producer.published_frames(), 2);
        producer.finish().unwrap();

        assert_eq!(
            source.by_ref().collect::<Vec<_>>(),
            vec![0.1, -0.1, 0.2, -0.2]
        );
        assert_eq!(cue_receiver.recv().unwrap(), cue(0, 10));
        assert_eq!(cue_receiver.recv().unwrap(), cue(2, 20));
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn progressive_cues_do_not_consume_bounded_audio_capacity() {
        let (cue_sender, cue_receiver) = mpsc::channel();
        let (mut producer, mut source, ticket) = ProgressivePlaybackSource::new(
            Box::new(move |cue| cue_sender.send(cue).unwrap()),
            CancellationToken::new(),
            CancellationToken::new(),
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        for identifier in 0..16 {
            producer.push_cues(vec![cue(0, identifier)]).unwrap();
        }
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1]))
            .unwrap();
        producer.finish().unwrap();

        assert_eq!(source.by_ref().collect::<Vec<_>>(), vec![0.1, -0.1]);
        assert_eq!(
            cue_receiver.try_iter().collect::<Vec<_>>(),
            (0..16)
                .map(|identifier| cue(0, identifier))
                .collect::<Vec<_>>()
        );
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn progressive_control_attaches_only_after_three_audio_windows() {
        let (speech_sink, _speech_output) = Sink::new_idle();
        let speech_sink = Arc::new(speech_sink);
        let (tone_sink, _tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(speech_sink.clone()),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let (mut producer, _ticket) = control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                |_| {},
                CancellationToken::new(),
                || true,
            )
            .unwrap()
            .unwrap();

        for expected_windows in 1..=PROGRESSIVE_PLAYBACK_PREBUFFER_WINDOWS {
            producer
                .push_audio(AudioBuffer::new(vec![0.1, -0.1]))
                .unwrap();
            assert_eq!(
                speech_sink.len(),
                usize::from(expected_windows == PROGRESSIVE_PLAYBACK_PREBUFFER_WINDOWS)
            );
        }
        producer.finish().unwrap();
    }

    #[test]
    fn progressive_control_attaches_a_short_stream_at_completion() {
        let (speech_sink, _speech_output) = Sink::new_idle();
        let speech_sink = Arc::new(speech_sink);
        let (tone_sink, _tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(speech_sink.clone()),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let (mut producer, _ticket) = control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                |_| {},
                CancellationToken::new(),
                || true,
            )
            .unwrap()
            .unwrap();
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1]))
            .unwrap();
        assert_eq!(speech_sink.len(), 0);

        producer.finish().unwrap();

        assert_eq!(speech_sink.len(), 1);
    }

    #[test]
    fn progressive_control_does_not_attach_after_stop_during_priming() {
        let (speech_sink, _speech_output) = Sink::new_idle();
        let speech_sink = Arc::new(speech_sink);
        let (tone_sink, _tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(speech_sink.clone()),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let (mut producer, ticket) = control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                |_| {},
                CancellationToken::new(),
                || true,
            )
            .unwrap()
            .unwrap();

        control.stop(StreamType::Speech);
        assert!(matches!(
            producer.push_audio(AudioBuffer::new(vec![0.1, -0.1])),
            Err(AudioError::PlaybackError(_))
        ));
        drop(producer);

        assert_eq!(speech_sink.len(), 0);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn progressive_source_reports_an_unfinished_producer_as_cancelled() {
        let (producer, mut source, ticket) = ProgressivePlaybackSource::new(
            Box::new(|_| {}),
            CancellationToken::new(),
            CancellationToken::new(),
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        drop(producer);

        assert_eq!(source.next(), None);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn progressive_source_cancellation_interrupts_an_empty_stream() {
        let cancellation = CancellationToken::new();
        let (_producer, mut source, ticket) = ProgressivePlaybackSource::new(
            Box::new(|_| {}),
            cancellation.clone(),
            CancellationToken::new(),
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        cancellation.cancel();

        assert_eq!(source.next(), None);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn progressive_producer_rejects_empty_or_late_frames() {
        let (mut producer, _source, _ticket) = ProgressivePlaybackSource::new(
            Box::new(|_| {}),
            CancellationToken::new(),
            CancellationToken::new(),
            Some(SPEECH_STOP_FADE_FRAMES),
        );
        assert!(matches!(
            producer.push_audio(AudioBuffer::empty()),
            Err(AudioError::InvalidFormat(_))
        ));
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1]))
            .unwrap();
        assert!(matches!(
            producer.push_cues(vec![cue(0, 10)]),
            Err(AudioError::InvalidFormat(_))
        ));
    }

    #[test]
    fn cloned_tickets_observe_the_same_terminal_state() {
        let (mut source, ticket) = TrackedBufferSource::new(vec![0.1, -0.1]);
        let barrier = ticket.clone();

        assert_eq!(source.by_ref().collect::<Vec<_>>(), vec![0.1, -0.1]);
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        assert_eq!(barrier.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn scheduled_playback_waits_for_every_guard() {
        let state = Arc::new(ScheduledPlaybackState::default());
        let first = state.begin();
        let second = state.begin();
        let waiting = state.clone();
        let (sender, receiver) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            waiting.wait();
            let _ = sender.send(());
        });

        drop(first);
        assert!(receiver.try_recv().is_err());
        drop(second);
        assert!(receiver.recv_timeout(Duration::from_secs(1)).is_ok());
        waiter.join().unwrap();
    }

    #[test]
    fn speech_stop_rotates_the_stream_cancellation_generation() {
        let (speech_sink, _speech_output) = Sink::new_idle();
        let (tone_sink, _tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(Arc::new(speech_sink)),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let initial = control.speech_stop_cancellation.lock().unwrap().clone();

        control.stop(StreamType::Speech);

        let replacement = control.speech_stop_cancellation.lock().unwrap().clone();
        assert!(initial.is_cancelled());
        assert!(!initial.same_token(&replacement));
        assert!(!replacement.is_cancelled());

        control.stop(StreamType::Speech);

        assert!(replacement.is_cancelled());
        assert!(!control
            .speech_stop_cancellation
            .lock()
            .unwrap()
            .is_cancelled());
    }

    #[test]
    fn tone_stop_rotates_the_stream_cancellation_generation() {
        let (speech_sink, _speech_output) = Sink::new_idle();
        let (tone_sink, _tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(Arc::new(speech_sink)),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let initial = control.tone_stop_cancellation.lock().unwrap().clone();

        control.stop(StreamType::Tone);

        let replacement = control.tone_stop_cancellation.lock().unwrap().clone();
        assert!(initial.is_cancelled());
        assert!(!initial.same_token(&replacement));
        assert!(!replacement.is_cancelled());

        control.stop(StreamType::Tone);

        assert!(replacement.is_cancelled());
        assert!(!control
            .tone_stop_cancellation
            .lock()
            .unwrap()
            .is_cancelled());
    }

    #[test]
    fn cancellation_fade_uses_smooth_endpoints() {
        let fade_frames = 101;

        assert_eq!(smooth_fade_gain(fade_frames, 0), 1.0);
        assert_eq!(smooth_fade_gain(fade_frames, fade_frames - 1), 0.0);
        assert!(smooth_fade_gain(fade_frames, 25) > 0.75);
        assert!(smooth_fade_gain(fade_frames, 75) < 0.25);
        assert!((0..fade_frames - 1).all(|frame| {
            smooth_fade_gain(fade_frames, frame) >= smooth_fade_gain(fade_frames, frame + 1)
        }));
    }

    #[test]
    fn tone_stop_fades_the_active_sink_source_to_zero() {
        let channels = CHANNELS as usize;
        let fade_samples = TONE_STOP_FADE_FRAMES * channels;
        let (speech_sink, _speech_output) = Sink::new_idle();
        let (tone_sink, mut tone_output) = Sink::new_idle();
        let (sound_sink, _sound_output) = Sink::new_idle();
        let control = AudioControl::new(
            ManagedSink::Device(Arc::new(speech_sink)),
            ManagedSink::Device(Arc::new(tone_sink)),
            ManagedSink::Device(Arc::new(sound_sink)),
            4,
            4,
            4,
        );
        let tone = AudioBuffer::new(vec![1.0; fade_samples + 2 * channels]);
        assert!(control.queue(StreamType::Tone, &tone).unwrap());
        assert_eq!(tone_output.next(), Some(1.0));
        assert_eq!(tone_output.next(), Some(1.0));

        control.stop(StreamType::Tone);

        let tail = tone_output.by_ref().take(fade_samples).collect::<Vec<_>>();
        assert_eq!(tail.len(), fade_samples);
        assert_eq!(tail.first(), Some(&1.0));
        assert_eq!(tail.last(), Some(&0.0));
    }

    #[test]
    fn null_backend_consumes_tracked_audio_and_cues_without_a_device() {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let buffer = AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]);

        let ticket = control
            .queue_tracked_with_cues(
                StreamType::Speech,
                &buffer,
                vec![cue(0, 10), cue(2, 20)],
                cue_sender,
            )
            .unwrap()
            .unwrap();

        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        assert_eq!(cue_receiver.recv().unwrap(), cue(0, 10));
        assert_eq!(cue_receiver.recv().unwrap(), cue(2, 20));
        control.drain();
        assert!(!control.is_playing(StreamType::Speech));
    }

    #[test]
    fn null_backend_consumes_progressive_speech_without_a_device() {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let cancellation = CancellationToken::new();
        let (mut producer, ticket) = control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                move |cue| cue_sender.send(cue).unwrap(),
                cancellation,
                || true,
            )
            .unwrap()
            .unwrap();

        producer.push_cues(vec![cue(0, 30)]).unwrap();
        producer
            .push_audio(AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]))
            .unwrap();
        producer.finish().unwrap();

        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
        assert_eq!(cue_receiver.recv().unwrap(), cue(0, 30));
        control.drain();
        assert!(!control.is_playing(StreamType::Speech));
    }

    #[test]
    fn null_backend_shutdown_interrupts_an_idle_progressive_source() {
        let streams = AudioStreams::new_with_backend(4, 4, 4, AudioBackend::Null).unwrap();
        let control = streams.control();
        let (mut producer, ticket) = control
            .queue_progressive_speech_with_cue_callback_cancellable_if(
                |_| {},
                CancellationToken::new(),
                || true,
            )
            .unwrap()
            .unwrap();
        drop(control);

        drop(streams);

        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
        assert!(matches!(
            producer.push_audio(AudioBuffer::new(vec![0.1, -0.1])),
            Err(AudioError::PlaybackError(_))
        ));
    }

    #[test]
    fn tracked_source_reports_early_drop_as_cancellation() {
        let (mut source, ticket) = TrackedBufferSource::new(vec![0.1, -0.1]);

        assert_eq!(source.next(), Some(0.1));
        drop(source);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn unstarted_tracked_source_stops_immediately_when_cancelled() {
        let cancellation = CancellationToken::new();
        let (mut source, ticket) =
            TrackedBufferSource::new_cancellable(vec![0.1, -0.1, 0.2, -0.2], cancellation.clone());

        cancellation.cancel();

        assert_eq!(source.size_hint(), (0, Some(0)));
        assert_eq!(source.current_frame_len(), Some(0));
        assert_eq!(source.next(), None);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn active_tracked_source_fades_to_zero_when_cancelled() {
        let channels = CHANNELS as usize;
        let fade_samples = SPEECH_STOP_FADE_FRAMES * channels;
        let cancellation = CancellationToken::new();
        let (mut source, ticket) = TrackedBufferSource::new_cancellable(
            vec![1.0; fade_samples + 2 * channels],
            cancellation.clone(),
        );

        assert_eq!(source.next(), Some(1.0));
        assert_eq!(source.next(), Some(1.0));
        cancellation.cancel();

        assert_eq!(source.size_hint(), (0, Some(fade_samples)));
        assert_eq!(source.current_frame_len(), Some(fade_samples));
        let tail = source.by_ref().collect::<Vec<_>>();
        assert_eq!(tail.len(), fade_samples);
        assert!(tail
            .chunks_exact(channels)
            .all(|frame| frame.iter().all(|sample| *sample == frame[0])));
        assert_eq!(tail.first(), Some(&1.0));
        assert_eq!(tail.last(), Some(&0.0));
        assert!(tail
            .chunks_exact(channels)
            .map(|frame| frame[0])
            .collect::<Vec<_>>()
            .windows(2)
            .all(|frames| frames[0] >= frames[1]));
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn stream_stop_fades_active_untracked_audio_but_discards_queued_audio() {
        let channels = CHANNELS as usize;
        let fade_samples = TONE_STOP_FADE_FRAMES * channels;
        let cancellation = CancellationToken::new();
        let mut active = SmoothStopBufferSource::new(
            vec![1.0; fade_samples + 2 * channels],
            cancellation.clone(),
            TONE_STOP_FADE_FRAMES,
        );
        let mut queued = SmoothStopBufferSource::new(
            vec![1.0; 2 * channels],
            cancellation.clone(),
            TONE_STOP_FADE_FRAMES,
        );

        assert_eq!(active.next(), Some(1.0));
        assert_eq!(active.next(), Some(1.0));
        cancellation.cancel();

        let tail = active.by_ref().collect::<Vec<_>>();
        assert_eq!(tail.len(), fade_samples);
        assert_eq!(tail.first(), Some(&1.0));
        assert_eq!(tail.last(), Some(&0.0));
        assert_eq!(queued.next(), None);
    }

    #[test]
    fn token_cancellation_is_scoped_across_active_and_queued_sources() {
        let channels = CHANNELS as usize;
        let fade_samples = SPEECH_STOP_FADE_FRAMES * channels;
        let navigation = CancellationToken::new();
        let review = CancellationToken::new();
        let (mut active, active_ticket) = TrackedBufferSource::new_cancellable(
            vec![1.0; fade_samples + 2 * channels],
            navigation.clone(),
        );
        let (mut queued, queued_ticket) =
            TrackedBufferSource::new_cancellable(vec![0.3, -0.3], navigation.clone());
        let (mut protected, protected_ticket) = TrackedBufferSource::new(vec![0.5, -0.5]);
        let (mut other_domain, other_ticket) =
            TrackedBufferSource::new_cancellable(vec![0.7, -0.7], review.clone());

        assert_eq!(active.next(), Some(1.0));
        assert_eq!(active.next(), Some(1.0));
        navigation.cancel();

        let active_tail = active.by_ref().collect::<Vec<_>>();
        assert_eq!(active_tail.len(), fade_samples);
        assert_eq!(active_tail.last(), Some(&0.0));
        assert_eq!(queued.next(), None);
        assert_eq!(protected.by_ref().collect::<Vec<_>>(), vec![0.5, -0.5]);
        assert_eq!(other_domain.by_ref().collect::<Vec<_>>(), vec![0.7, -0.7]);
        assert_eq!(active_ticket.wait(), PlaybackStatus::Cancelled);
        assert_eq!(queued_ticket.wait(), PlaybackStatus::Cancelled);
        assert_eq!(protected_ticket.wait(), PlaybackStatus::Completed);
        assert_eq!(other_ticket.wait(), PlaybackStatus::Completed);
        assert!(!review.is_cancelled());
    }

    #[test]
    fn rapid_replacement_burst_leaves_only_the_latest_source_playable() {
        let mut current: Option<CancellationToken> = None;
        let mut playback = Vec::new();

        for request in 0..256 {
            if let Some(previous) = current.take() {
                previous.cancel();
            }
            let cancellation = CancellationToken::new();
            let (source, ticket) = TrackedBufferSource::new_cancellable(
                vec![request as f32, -(request as f32)],
                cancellation.clone(),
            );
            playback.push((source, ticket));
            current = Some(cancellation);
        }

        let last = playback.len() - 1;
        for (index, (mut source, ticket)) in playback.into_iter().enumerate() {
            if index == last {
                assert_eq!(source.by_ref().collect::<Vec<_>>(), vec![255.0, -255.0]);
                assert_eq!(ticket.wait(), PlaybackStatus::Completed);
            } else {
                assert_eq!(source.next(), None);
                assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
            }
        }
    }

    #[test]
    fn tracked_source_reports_only_one_terminal_state() {
        let (mut source, ticket) = TrackedBufferSource::new(vec![0.1]);

        assert_eq!(source.next(), Some(0.1));
        assert_eq!(source.next(), None);
        assert_eq!(source.next(), None);
        drop(source);
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn tracked_source_emits_ordered_cues_at_frame_boundaries() {
        let cues = prepare_playback_cues(
            vec![cue(2, 20), cue(1, 10), cue(3, 30), cue(1, 11), cue(0, 0)],
            3,
        )
        .unwrap();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let (mut source, ticket) =
            TrackedBufferSource::new_with_cues(vec![0.1; 6], cues, cue_sender);

        assert_eq!(source.next(), Some(0.1));
        assert_eq!(cue_receiver.try_iter().collect::<Vec<_>>(), vec![cue(0, 0)]);
        assert_eq!(source.next(), Some(0.1));
        assert!(cue_receiver.try_iter().next().is_none());

        assert_eq!(source.next(), Some(0.1));
        assert_eq!(
            cue_receiver.try_iter().collect::<Vec<_>>(),
            vec![cue(1, 10), cue(1, 11)]
        );
        assert_eq!(source.next(), Some(0.1));
        assert_eq!(source.next(), Some(0.1));
        assert_eq!(
            cue_receiver.try_iter().collect::<Vec<_>>(),
            vec![cue(2, 20)]
        );
        assert_eq!(source.next(), Some(0.1));

        assert_eq!(source.next(), None);
        assert_eq!(
            cue_receiver.try_iter().collect::<Vec<_>>(),
            vec![cue(3, 30)]
        );
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }

    #[test]
    fn tracked_source_drops_unreached_cues_on_cancellation() {
        let cues = prepare_playback_cues(vec![cue(0, 0), cue(1, 10), cue(2, 20)], 3).unwrap();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let (mut source, ticket) =
            TrackedBufferSource::new_with_cues(vec![0.1; 6], cues, cue_sender);

        assert_eq!(source.next(), Some(0.1));
        drop(source);

        assert_eq!(cue_receiver.try_iter().collect::<Vec<_>>(), vec![cue(0, 0)]);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn token_cancellation_suppresses_unreached_cues() {
        let channels = CHANNELS as usize;
        let frame_count = SPEECH_STOP_FADE_FRAMES + 2;
        let cues =
            prepare_playback_cues(vec![cue(0, 0), cue(1, 10), cue(2, 20)], frame_count).unwrap();
        let cancellation = CancellationToken::new();
        let (cue_sender, cue_receiver) = mpsc::channel();
        let (mut source, ticket) = TrackedBufferSource::new_with_cue_callback_cancellable(
            vec![0.1; frame_count * channels],
            cues,
            move |cue| {
                let _ = cue_sender.send(cue);
            },
            cancellation.clone(),
        );

        assert_eq!(source.next(), Some(0.1));
        assert_eq!(source.next(), Some(0.1));
        assert_eq!(cue_receiver.try_iter().collect::<Vec<_>>(), vec![cue(0, 0)]);
        cancellation.cancel();

        assert_eq!(
            source.by_ref().collect::<Vec<_>>().len(),
            SPEECH_STOP_FADE_FRAMES * channels
        );
        assert!(cue_receiver.try_iter().next().is_none());
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
    }

    #[test]
    fn playback_cues_reject_offsets_beyond_the_buffer() {
        let error = prepare_playback_cues(vec![cue(4, 40)], 3).unwrap_err();

        assert!(matches!(error, AudioError::InvalidFormat(_)));
        assert!(error
            .to_string()
            .contains("offset 4 exceeds buffer frame count 3"));
    }

    #[test]
    fn tracked_source_invokes_cue_callback_without_changing_completion() {
        let cues = prepare_playback_cues(vec![cue(1, 10)], 2).unwrap();
        let (sender, receiver) = mpsc::channel();
        let (mut source, ticket) =
            TrackedBufferSource::new_with_cue_callback(vec![0.1; 4], cues, move |cue| {
                let _ = sender.send(cue);
            });

        assert_eq!(source.by_ref().collect::<Vec<_>>(), vec![0.1; 4]);
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), vec![cue(1, 10)]);
        assert_eq!(ticket.wait(), PlaybackStatus::Completed);
    }
}
