//! Audio Output
//!
//! Wraps rodio for audio playback. Takes a processed AudioBuffer and plays it.

use crate::AudioError;
use crate::buffer::{AudioBuffer, CHANNELS, SAMPLE_RATE};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::Arc;
use std::time::Duration;

/// Audio output device for playback via rodio.
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
