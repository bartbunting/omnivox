//! TTS Engine Abstraction
//!
//! Platform-agnostic TTS trait and implementations for different platforms.

use thiserror::Error;

pub mod espeak;
pub mod macos;
pub mod windows;

/// TTS engine errors
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("Voice not found: {0}")]
    VoiceNotFound(String),

    #[error("Synthesis failed: {0}")]
    SynthesisFailed(String),

    #[error("Engine not available")]
    NotAvailable,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

/// Audio buffer containing PCM samples
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Interleaved PCM samples (f32, range -1.0 to 1.0)
    pub samples: Vec<f32>,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
}

/// Standard output sample rate
pub const STANDARD_SAMPLE_RATE: u32 = 44100;
/// Standard output channel count (stereo)
pub const STANDARD_CHANNELS: u16 = 2;

impl AudioBuffer {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
        }
    }

    /// Create an empty stereo buffer at the standard sample rate
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: STANDARD_SAMPLE_RATE,
            channels: STANDARD_CHANNELS,
        }
    }

    /// Get duration in seconds
    pub fn duration(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / (self.sample_rate as f32 * self.channels as f32)
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Get number of frames
    pub fn frame_count(&self) -> usize {
        if self.channels == 0 {
            return 0;
        }
        self.samples.len() / self.channels as usize
    }

    /// Convert i16 PCM samples to f32 (-1.0 to 1.0)
    pub fn from_i16(samples: &[i16], sample_rate: u32, channels: u16) -> Self {
        let f32_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        Self::new(f32_samples, sample_rate, channels)
    }

    /// Convert mono to stereo by duplicating each sample
    pub fn to_stereo(&self) -> Self {
        if self.channels == 2 {
            return self.clone();
        }
        assert_eq!(self.channels, 1, "Only mono to stereo conversion is supported");
        let mut stereo = Vec::with_capacity(self.samples.len() * 2);
        for &sample in &self.samples {
            stereo.push(sample);
            stereo.push(sample);
        }
        Self::new(stereo, self.sample_rate, 2)
    }

    /// Resample to a target sample rate using linear interpolation
    pub fn resample(&self, target_rate: u32) -> Self {
        if self.sample_rate == target_rate {
            return self.clone();
        }
        if self.samples.is_empty() {
            return Self::new(Vec::new(), target_rate, self.channels);
        }

        let channels = self.channels as usize;
        let frame_count = self.frame_count();
        if frame_count == 0 {
            return Self::new(Vec::new(), target_rate, self.channels);
        }

        let ratio = target_rate as f64 / self.sample_rate as f64;
        let new_frame_count = (frame_count as f64 * ratio) as usize;
        let mut resampled = Vec::with_capacity(new_frame_count * channels);

        for i in 0..new_frame_count {
            let src_pos = i as f64 / ratio;
            let src_idx = src_pos as usize;
            let frac = src_pos - src_idx as f64;

            for ch in 0..channels {
                let s0 = self.samples[src_idx * channels + ch];
                let s1 = if src_idx + 1 < frame_count {
                    self.samples[(src_idx + 1) * channels + ch]
                } else {
                    s0
                };
                let interpolated = s0 as f64 * (1.0 - frac) + s1 as f64 * frac;
                resampled.push(interpolated as f32);
            }
        }

        Self::new(resampled, target_rate, self.channels)
    }

    /// Convert to standard format: stereo f32 @ 44100Hz
    pub fn to_standard_format(&self) -> Self {
        let stereo = self.to_stereo();
        stereo.resample(STANDARD_SAMPLE_RATE)
    }
}

/// Voice information
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceInfo {
    /// Unique voice identifier
    pub identifier: String,
    /// Display name
    pub name: String,
    /// Language code (e.g., "en-US")
    pub language: String,
    /// Voice quality level
    pub quality: VoiceQuality,
}

/// Voice quality levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceQuality {
    /// Compact/basic quality
    Compact,
    /// Enhanced quality
    Enhanced,
    /// Premium/highest quality
    Premium,
}

/// TTS synthesis settings
#[derive(Debug, Clone)]
pub struct TtsSettings {
    /// Voice identifier
    pub voice: String,
    /// Speech rate (0.0 to 1.0, 0.5 = normal)
    pub rate: f32,
    /// Pitch multiplier (0.5 to 2.0, 1.0 = normal)
    pub pitch: f32,
    /// Volume (0.0 to 1.0)
    pub volume: f32,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            voice: String::from("en-US"),
            rate: 0.5,
            pitch: 1.0,
            volume: 1.0,
        }
    }
}

/// Platform-agnostic TTS engine trait
pub trait TtsEngine: Send + Sync {
    /// Synthesize text to an audio buffer
    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError>;

    /// Stop current synthesis
    fn stop(&self);

    /// Check if currently synthesizing
    fn is_speaking(&self) -> bool;

    /// List available voices
    fn available_voices(&self) -> Vec<VoiceInfo>;

    /// Get voice info by identifier
    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_buffer_new() {
        let buf = AudioBuffer::new(vec![0.5, -0.5, 0.25, -0.25], 44100, 2);
        assert_eq!(buf.sample_rate, 44100);
        assert_eq!(buf.channels, 2);
        assert_eq!(buf.samples.len(), 4);
    }

    #[test]
    fn test_audio_buffer_empty() {
        let buf = AudioBuffer::empty();
        assert!(buf.is_empty());
        assert_eq!(buf.sample_rate, STANDARD_SAMPLE_RATE);
        assert_eq!(buf.channels, STANDARD_CHANNELS);
        assert_eq!(buf.duration(), 0.0);
    }

    #[test]
    fn test_audio_buffer_duration() {
        // 44100 samples at 44100Hz mono = 1.0 seconds
        let samples: Vec<f32> = vec![0.0; 44100];
        let buf = AudioBuffer::new(samples, 44100, 1);
        assert!((buf.duration() - 1.0).abs() < 0.001);

        // 88200 samples at 44100Hz stereo = 1.0 seconds
        let samples: Vec<f32> = vec![0.0; 88200];
        let buf = AudioBuffer::new(samples, 44100, 2);
        assert!((buf.duration() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_audio_buffer_frame_count() {
        let buf = AudioBuffer::new(vec![0.0; 100], 44100, 2);
        assert_eq!(buf.frame_count(), 50);

        let buf = AudioBuffer::new(vec![0.0; 100], 44100, 1);
        assert_eq!(buf.frame_count(), 100);
    }

    #[test]
    fn test_from_i16() {
        let buf = AudioBuffer::from_i16(&[0, 16384, -16384, 32767], 22050, 1);
        assert_eq!(buf.sample_rate, 22050);
        assert_eq!(buf.channels, 1);
        assert_eq!(buf.samples.len(), 4);
        assert!((buf.samples[0] - 0.0).abs() < 0.001);
        assert!((buf.samples[1] - 0.5).abs() < 0.001);
        assert!((buf.samples[2] + 0.5).abs() < 0.001);
        assert!((buf.samples[3] - (32767.0 / 32768.0)).abs() < 0.001);
    }

    #[test]
    fn test_to_stereo() {
        let mono = AudioBuffer::new(vec![0.1, 0.2, 0.3], 44100, 1);
        let stereo = mono.to_stereo();
        assert_eq!(stereo.channels, 2);
        assert_eq!(stereo.samples.len(), 6);
        assert_eq!(stereo.samples, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn test_to_stereo_already_stereo() {
        let stereo = AudioBuffer::new(vec![0.1, 0.2, 0.3, 0.4], 44100, 2);
        let result = stereo.to_stereo();
        assert_eq!(result.channels, 2);
        assert_eq!(result.samples.len(), 4);
    }

    #[test]
    fn test_resample_same_rate() {
        let buf = AudioBuffer::new(vec![0.1, 0.2, 0.3], 44100, 1);
        let resampled = buf.resample(44100);
        assert_eq!(resampled.samples, buf.samples);
    }

    #[test]
    fn test_resample_upsampling() {
        // 22050 -> 44100 should roughly double the sample count
        let samples: Vec<f32> = (0..100).map(|i| (i as f32) / 100.0).collect();
        let buf = AudioBuffer::new(samples, 22050, 1);
        let resampled = buf.resample(44100);
        assert_eq!(resampled.sample_rate, 44100);
        assert!(resampled.samples.len() >= 190 && resampled.samples.len() <= 210);
    }

    #[test]
    fn test_resample_downsampling() {
        // 44100 -> 22050 should roughly halve the sample count
        let samples: Vec<f32> = (0..200).map(|i| (i as f32) / 200.0).collect();
        let buf = AudioBuffer::new(samples, 44100, 1);
        let resampled = buf.resample(22050);
        assert_eq!(resampled.sample_rate, 22050);
        assert!(resampled.samples.len() >= 90 && resampled.samples.len() <= 110);
    }

    #[test]
    fn test_to_standard_format() {
        // Mono 22050Hz -> stereo 44100Hz
        let buf = AudioBuffer::new(vec![0.5; 100], 22050, 1);
        let standard = buf.to_standard_format();
        assert_eq!(standard.sample_rate, STANDARD_SAMPLE_RATE);
        assert_eq!(standard.channels, STANDARD_CHANNELS);
    }

    #[test]
    fn test_resample_empty() {
        let buf = AudioBuffer::new(Vec::new(), 22050, 1);
        let resampled = buf.resample(44100);
        assert!(resampled.is_empty());
        assert_eq!(resampled.sample_rate, 44100);
    }

    #[test]
    fn test_duration_zero_rate() {
        let buf = AudioBuffer::new(vec![0.0; 100], 0, 1);
        assert_eq!(buf.duration(), 0.0);
    }

    #[test]
    fn test_tts_settings_default() {
        let settings = TtsSettings::default();
        assert_eq!(settings.voice, "en-US");
        assert_eq!(settings.rate, 0.5);
        assert_eq!(settings.pitch, 1.0);
        assert_eq!(settings.volume, 1.0);
    }
}
