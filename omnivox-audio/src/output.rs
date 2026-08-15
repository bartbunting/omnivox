//! Audio Output
//!
//! Wraps rodio for audio playback. Provides both single-shot playback
//! (`AudioOutput`) and concurrent multi-stream playback (`AudioStreams`).
//!
//! The key type for multi-threaded use is `AudioControl` -- a `Send + Sync`
//! handle to the three audio sinks. `AudioStreams` owns the `OutputStream`
//! drop guard (which is `!Send` on some platforms) and should stay on the
//! thread where it was created. `AudioControl` can be cloned via
//! `AudioStreams::control()` and sent to other threads (e.g. a synthesis
//! worker) to queue or stop audio independently.

use crate::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use crate::AudioError;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tracing::debug;

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
/// `Sink` is `Send + Sync` in rodio, so this type is automatically `Send + Sync`.
#[derive(Clone)]
pub struct AudioControl {
    speech_sink: Arc<Sink>,
    tone_sink: Arc<Sink>,
    sound_sink: Arc<Sink>,
    speech_max: usize,
    tone_max: usize,
    sound_max: usize,
    schedule_generations: Arc<[AtomicU64; 3]>,
    stream_gates: Arc<[Mutex<()>; 3]>,
    scheduled_playback: Arc<ScheduledPlaybackState>,
}

impl AudioControl {
    fn sink_and_max(&self, stream: StreamType) -> (&Arc<Sink>, usize) {
        match stream {
            StreamType::Speech => (&self.speech_sink, self.speech_max),
            StreamType::Tone => (&self.tone_sink, self.tone_max),
            StreamType::Sound => (&self.sound_sink, self.sound_max),
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

        let source = BufferSource::new(buffer.samples.clone());
        sink.append(source);
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
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(None);
        }
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(None);
        };

        let (source, ticket) = TrackedBufferSource::new(buffer.samples.clone());
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
        self.queue_stream_after_if(StreamType::Sound, buffer, barriers, predicate)
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
        if buffer.is_empty() {
            return Ok(None);
        }
        if barriers.is_empty() {
            return self.queue_tracked_if(stream, buffer, predicate);
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
                    || control.schedule_generations[stream_index].load(Ordering::Acquire)
                        != generation
                {
                    completion.report(PlaybackStatus::Cancelled);
                    return;
                }
                let overlay = AudioBuffer::new(samples);
                let status = match control.queue_tracked_if(stream, &overlay, || {
                    control.schedule_generations[stream_index].load(Ordering::Acquire) == generation
                }) {
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

        let (source, ticket) =
            TrackedBufferSource::new_with_cues(buffer.samples.clone(), cues, cue_sender);
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
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        if !predicate() {
            return Ok(None);
        }
        let cues = prepare_playback_cues(cues, buffer.frame_count())?;
        let Some(sink) = self.prepare_queue(stream, buffer) else {
            return Ok(None);
        };

        let (source, ticket) =
            TrackedBufferSource::new_with_cue_callback(buffer.samples.clone(), cues, on_cue);
        sink.append(source);
        sink.play();
        Ok(Some(ticket))
    }

    fn prepare_queue(&self, stream: StreamType, buffer: &AudioBuffer) -> Option<&Arc<Sink>> {
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

    /// Stop a specific stream, clearing all queued and playing audio.
    pub fn stop(&self, stream: StreamType) {
        let _gate = self.stream_gates[stream_index(stream)].lock().unwrap();
        self.schedule_generations[stream_index(stream)].fetch_add(1, Ordering::AcqRel);
        let (sink, _) = self.sink_and_max(stream);
        sink.clear();
        sink.play();
    }

    /// Stop all streams immediately.
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
/// Owns the `OutputStream` drop guard and delegates all audio operations to an
/// inner `Arc<AudioControl>`. Call `control()` to get a shareable handle for
/// use from other threads (e.g. synthesis worker thread).
pub struct AudioStreams {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    control: Arc<AudioControl>,
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
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| AudioError::DeviceNotFound(format!("default device: {}", e)))?;

        let speech_sink = Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("speech sink: {}", e)))?,
        );
        let tone_sink = Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("tone sink: {}", e)))?,
        );
        let sound_sink = Arc::new(
            Sink::try_new(&stream_handle)
                .map_err(|e| AudioError::PlaybackError(format!("sound sink: {}", e)))?,
        );

        let control = Arc::new(AudioControl {
            speech_sink,
            tone_sink,
            sound_sink,
            speech_max: speech_max_depth,
            tone_max: tone_max_depth,
            sound_max: sound_max_depth,
            schedule_generations: Arc::new([
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ]),
            stream_gates: Arc::new([Mutex::new(()), Mutex::new(()), Mutex::new(())]),
            scheduled_playback: Arc::new(ScheduledPlaybackState::default()),
        });

        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            control,
        })
    }

    /// Get a thread-safe handle to the audio controls.
    ///
    /// The returned `Arc<AudioControl>` is `Send + Sync` and can be cloned and
    /// sent to the synthesis worker thread. The `AudioStreams` (and its
    /// `OutputStream` drop guard) must remain alive for audio to work.
    pub fn control(&self) -> Arc<AudioControl> {
        self.control.clone()
    }

    /// Queue an audio buffer on the given stream.
    pub fn queue(&self, stream: StreamType, buffer: &AudioBuffer) -> Result<bool, AudioError> {
        self.control.queue(stream, buffer)
    }

    /// Stop a specific stream, clearing all queued and playing audio.
    pub fn stop(&self, stream: StreamType) {
        self.control.stop(stream)
    }

    /// Stop all streams immediately.
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

/// Buffer source that reports natural exhaustion and cancellation distinctly.
struct TrackedBufferSource {
    inner: BufferSource,
    completion: PlaybackCompletion,
    cues: Vec<PlaybackCue>,
    next_cue: usize,
    cue_sender: Option<Sender<PlaybackCue>>,
    cue_callback: Option<Box<dyn FnMut(PlaybackCue) + Send>>,
}

impl TrackedBufferSource {
    fn new(samples: Vec<f32>) -> (Self, PlaybackTicket) {
        let (completion, ticket) = PlaybackCompletion::pair();
        (
            Self {
                inner: BufferSource::new(samples),
                completion,
                cues: Vec::new(),
                next_cue: 0,
                cue_sender: None,
                cue_callback: None,
            },
            ticket,
        )
    }

    fn new_with_cues(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        cue_sender: Sender<PlaybackCue>,
    ) -> (Self, PlaybackTicket) {
        let (completion, ticket) = PlaybackCompletion::pair();
        (
            Self {
                inner: BufferSource::new(samples),
                completion,
                cues,
                next_cue: 0,
                cue_sender: Some(cue_sender),
                cue_callback: None,
            },
            ticket,
        )
    }

    fn new_with_cue_callback<F>(
        samples: Vec<f32>,
        cues: Vec<PlaybackCue>,
        on_cue: F,
    ) -> (Self, PlaybackTicket)
    where
        F: FnMut(PlaybackCue) + Send + 'static,
    {
        let (completion, ticket) = PlaybackCompletion::pair();
        (
            Self {
                inner: BufferSource::new(samples),
                completion,
                cues,
                next_cue: 0,
                cue_sender: None,
                cue_callback: Some(Box::new(on_cue)),
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
        self.report_cues_at_current_frame();
        let sample = self.inner.next();
        if sample.is_none() {
            self.report(PlaybackStatus::Completed);
        }
        sample
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
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
        self.inner.current_frame_len()
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
    fn tracked_source_reports_early_drop_as_cancellation() {
        let (mut source, ticket) = TrackedBufferSource::new(vec![0.1, -0.1]);

        assert_eq!(source.next(), Some(0.1));
        drop(source);
        assert_eq!(ticket.wait(), PlaybackStatus::Cancelled);
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
