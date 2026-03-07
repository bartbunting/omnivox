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

use crate::AudioError;
use crate::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::Arc;
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

/// Thread-safe handle to the three audio sinks.
///
/// Both the reader thread (for `stop_all` on `s` command) and the synthesis
/// worker thread (for queuing synthesized audio) hold an `Arc<AudioControl>`.
/// `Sink` is `Send + Sync` in rodio, so this type is automatically `Send + Sync`.
pub struct AudioControl {
    speech_sink: Arc<Sink>,
    tone_sink: Arc<Sink>,
    sound_sink: Arc<Sink>,
    speech_max: usize,
    tone_max: usize,
    sound_max: usize,
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
        if buffer.is_empty() {
            return Ok(false);
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

        let source = BufferSource::new(buffer.samples.clone());
        sink.append(source);
        sink.play();
        Ok(true)
    }

    /// Stop a specific stream, clearing all queued and playing audio.
    pub fn stop(&self, stream: StreamType) {
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
                sink: Arc::new(
                    Sink::try_new(&self.stream_handle).map_err(|e| {
                        AudioError::PlaybackError(format!("failed to create sink: {}", e))
                    })?,
                ),
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
        Some(Duration::from_secs_f64(
            frames as f64 / SAMPLE_RATE as f64,
        ))
    }
}
