//! Audio Buffer
//!
//! The canonical audio buffer format for omnivox: stereo f32 at 44100Hz.
//! All audio sources produce this format, and all consumers expect it.

/// Standard sample rate for all omnivox audio
pub const SAMPLE_RATE: u32 = 44100;

/// Standard channel count (stereo)
pub const CHANNELS: u16 = 2;

/// Audio buffer containing interleaved stereo f32 PCM samples at 44100Hz.
///
/// This is the universal format used throughout the omnivox audio pipeline.
/// All sources (TTS, tone generator, file loader) produce AudioBuffers,
/// and all consumers (effects, output) operate on them.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Interleaved stereo PCM samples (f32, range -1.0 to 1.0).
    /// Layout: [L0, R0, L1, R1, L2, R2, ...]
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// Create a new AudioBuffer from interleaved stereo samples.
    ///
    /// Panics if the number of samples is odd (not a multiple of 2).
    pub fn new(samples: Vec<f32>) -> Self {
        assert!(
            samples.len().is_multiple_of(CHANNELS as usize),
            "Sample count must be even for stereo interleaved data"
        );
        Self { samples }
    }

    /// Create an empty AudioBuffer.
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    /// Create a silent AudioBuffer of the given duration in seconds.
    pub fn silence(duration_secs: f32) -> Self {
        let frame_count = (duration_secs * SAMPLE_RATE as f32) as usize;
        let sample_count = frame_count * CHANNELS as usize;
        Self {
            samples: vec![0.0; sample_count],
        }
    }

    /// Get the sample rate (always 44100).
    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// Get the channel count (always 2).
    pub fn channels(&self) -> u16 {
        CHANNELS
    }

    /// Get duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.frame_count() as f32 / SAMPLE_RATE as f32
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Get number of frames (each frame is one sample per channel).
    pub fn frame_count(&self) -> usize {
        self.samples.len() / CHANNELS as usize
    }

    /// Get the left channel sample at the given frame index.
    pub fn left(&self, frame: usize) -> f32 {
        self.samples[frame * 2]
    }

    /// Get the right channel sample at the given frame index.
    pub fn right(&self, frame: usize) -> f32 {
        self.samples[frame * 2 + 1]
    }

    /// Set the left channel sample at the given frame index.
    pub fn set_left(&mut self, frame: usize, value: f32) {
        self.samples[frame * 2] = value;
    }

    /// Set the right channel sample at the given frame index.
    pub fn set_right(&mut self, frame: usize, value: f32) {
        self.samples[frame * 2 + 1] = value;
    }

    /// Append another AudioBuffer to this one.
    pub fn append(&mut self, other: &AudioBuffer) {
        self.samples.extend_from_slice(&other.samples);
    }

    /// Clamp all samples to the range [-1.0, 1.0].
    pub fn clamp(&mut self) {
        for sample in &mut self.samples {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    /// Get the peak absolute amplitude across all samples.
    pub fn peak_amplitude(&self) -> f32 {
        self.samples
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer() {
        let buf = AudioBuffer::new(vec![0.1, -0.1, 0.2, -0.2]);
        assert_eq!(buf.frame_count(), 2);
        assert_eq!(buf.samples.len(), 4);
        assert_eq!(buf.sample_rate(), SAMPLE_RATE);
        assert_eq!(buf.channels(), CHANNELS);
    }

    #[test]
    #[should_panic(expected = "Sample count must be even")]
    fn test_new_buffer_odd_samples_panics() {
        AudioBuffer::new(vec![0.1, -0.1, 0.2]);
    }

    #[test]
    fn test_empty_buffer() {
        let buf = AudioBuffer::empty();
        assert!(buf.is_empty());
        assert_eq!(buf.frame_count(), 0);
        assert_eq!(buf.duration_secs(), 0.0);
    }

    #[test]
    fn test_silence() {
        let buf = AudioBuffer::silence(1.0);
        assert_eq!(buf.frame_count(), SAMPLE_RATE as usize);
        assert_eq!(buf.samples.len(), SAMPLE_RATE as usize * 2);
        assert!(buf.samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_duration() {
        let buf = AudioBuffer::silence(0.5);
        let duration = buf.duration_secs();
        assert!((duration - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_left_right_access() {
        let buf = AudioBuffer::new(vec![0.5, -0.5, 0.3, -0.3]);
        assert_eq!(buf.left(0), 0.5);
        assert_eq!(buf.right(0), -0.5);
        assert_eq!(buf.left(1), 0.3);
        assert_eq!(buf.right(1), -0.3);
    }

    #[test]
    fn test_set_left_right() {
        let mut buf = AudioBuffer::new(vec![0.0, 0.0, 0.0, 0.0]);
        buf.set_left(0, 0.7);
        buf.set_right(0, -0.7);
        buf.set_left(1, 0.9);
        buf.set_right(1, -0.9);
        assert_eq!(buf.left(0), 0.7);
        assert_eq!(buf.right(0), -0.7);
        assert_eq!(buf.left(1), 0.9);
        assert_eq!(buf.right(1), -0.9);
    }

    #[test]
    fn test_append() {
        let mut buf1 = AudioBuffer::new(vec![0.1, 0.2]);
        let buf2 = AudioBuffer::new(vec![0.3, 0.4]);
        buf1.append(&buf2);
        assert_eq!(buf1.frame_count(), 2);
        assert_eq!(buf1.samples, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn test_clamp() {
        let mut buf = AudioBuffer::new(vec![1.5, -1.5, 0.5, -0.5]);
        buf.clamp();
        assert_eq!(buf.samples, vec![1.0, -1.0, 0.5, -0.5]);
    }

    #[test]
    fn test_peak_amplitude() {
        let buf = AudioBuffer::new(vec![0.3, -0.7, 0.5, -0.1]);
        assert_eq!(buf.peak_amplitude(), 0.7);
    }

    #[test]
    fn test_peak_amplitude_empty() {
        let buf = AudioBuffer::empty();
        assert_eq!(buf.peak_amplitude(), 0.0);
    }
}
