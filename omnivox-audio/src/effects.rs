//! Audio Effects
//!
//! Post-processing effects implementing the AudioEffect trait.
//! The pipeline is extensible for future effects (phaser, echo,
//! chorus, tremolo, reverb, pan).

use crate::AudioError;
use crate::buffer::{AudioBuffer, SAMPLE_RATE};
use crate::pipeline::AudioEffect;
use omnivox_core::ChannelMode;

/// Removes leading and trailing silence from an AudioBuffer.
///
/// Samples with absolute amplitude below the threshold are considered silent.
/// A small amount of padding is preserved to prevent harsh cuts.
pub struct SilenceTrimmer {
    /// Amplitude threshold below which samples are considered silence
    pub threshold: f32,
    /// Minimum padding to preserve at start/end in seconds
    pub padding_secs: f32,
}

impl SilenceTrimmer {
    /// Create a new SilenceTrimmer with default settings.
    ///
    /// Default threshold: 0.01, default padding: 5ms.
    pub fn new() -> Self {
        Self {
            threshold: 0.01,
            padding_secs: 0.005, // 5ms
        }
    }

    /// Create a SilenceTrimmer with custom threshold and padding.
    pub fn with_settings(threshold: f32, padding_secs: f32) -> Self {
        Self {
            threshold,
            padding_secs,
        }
    }
}

impl Default for SilenceTrimmer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for SilenceTrimmer {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
        if buffer.is_empty() {
            return Ok(());
        }

        let frame_count = buffer.frame_count();
        let padding_frames = (self.padding_secs * SAMPLE_RATE as f32) as usize;

        // Find first non-silent frame
        let mut first_sound = 0;
        for i in 0..frame_count {
            if buffer.left(i).abs() > self.threshold || buffer.right(i).abs() > self.threshold {
                first_sound = i;
                break;
            }
            if i == frame_count - 1 {
                // Entire buffer is silence
                buffer.samples.clear();
                return Ok(());
            }
        }

        // Find last non-silent frame
        let mut last_sound = frame_count - 1;
        for i in (0..frame_count).rev() {
            if buffer.left(i).abs() > self.threshold || buffer.right(i).abs() > self.threshold {
                last_sound = i;
                break;
            }
        }

        // Apply padding (but don't go beyond buffer bounds)
        let start_frame = first_sound.saturating_sub(padding_frames);
        let end_frame = (last_sound + padding_frames + 1).min(frame_count);

        // Only trim if there's actually silence to remove
        if start_frame > 0 || end_frame < frame_count {
            let start_sample = start_frame * 2;
            let end_sample = end_frame * 2;
            buffer.samples = buffer.samples[start_sample..end_sample].to_vec();
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "silence_trimmer"
    }
}

/// Scales all samples by a volume multiplier.
///
/// Output is clamped to [-1.0, 1.0] to prevent clipping.
pub struct VolumeAdjust {
    /// Volume multiplier (1.0 = unity gain)
    pub volume: f32,
}

impl VolumeAdjust {
    /// Create a new VolumeAdjust with the given volume level.
    pub fn new(volume: f32) -> Self {
        Self { volume }
    }
}

impl AudioEffect for VolumeAdjust {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
        for sample in &mut buffer.samples {
            *sample = (*sample * self.volume).clamp(-1.0, 1.0);
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "volume_adjust"
    }
}

/// Routes audio to left, right, or both channels.
///
/// Maps to the ChannelMode enum from omnivox-core.
pub struct ChannelRouter {
    /// Which channels to route audio to
    pub mode: ChannelMode,
}

impl ChannelRouter {
    /// Create a new ChannelRouter with the given mode.
    pub fn new(mode: ChannelMode) -> Self {
        Self { mode }
    }
}

impl AudioEffect for ChannelRouter {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
        match self.mode {
            ChannelMode::Both => {
                // Pass through unchanged
            }
            ChannelMode::Left => {
                // Zero out right channel
                for i in 0..buffer.frame_count() {
                    buffer.set_right(i, 0.0);
                }
            }
            ChannelMode::Right => {
                // Zero out left channel
                for i in 0..buffer.frame_count() {
                    buffer.set_left(i, 0.0);
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "channel_router"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SilenceTrimmer tests ---

    #[test]
    fn test_silence_trimmer_empty_buffer() {
        let trimmer = SilenceTrimmer::new();
        let mut buf = AudioBuffer::empty();
        trimmer.process(&mut buf).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_silence_trimmer_all_silence() {
        let trimmer = SilenceTrimmer::new();
        let mut buf = AudioBuffer::new(vec![0.0; 1000]);
        trimmer.process(&mut buf).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_silence_trimmer_no_silence() {
        let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);
        let samples = vec![0.5, -0.5, 0.3, -0.3, 0.1, -0.1];
        let original_len = samples.len();
        let mut buf = AudioBuffer::new(samples);
        trimmer.process(&mut buf).unwrap();
        assert_eq!(buf.samples.len(), original_len);
    }

    #[test]
    fn test_silence_trimmer_leading_silence() {
        let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);
        // 5 frames of silence, then 3 frames of sound
        let mut samples = vec![0.0; 10]; // 5 silent frames
        samples.extend_from_slice(&[0.5, -0.5, 0.3, -0.3, 0.1, -0.1]); // 3 sound frames
        let mut buf = AudioBuffer::new(samples);
        trimmer.process(&mut buf).unwrap();
        // Should have removed leading silence
        assert_eq!(buf.frame_count(), 3);
        assert_eq!(buf.left(0), 0.5);
    }

    #[test]
    fn test_silence_trimmer_trailing_silence() {
        let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);
        let mut samples = vec![0.5, -0.5, 0.3, -0.3]; // 2 sound frames
        samples.extend_from_slice(&[0.0; 10]); // 5 silent frames
        let mut buf = AudioBuffer::new(samples);
        trimmer.process(&mut buf).unwrap();
        assert_eq!(buf.frame_count(), 2);
    }

    #[test]
    fn test_silence_trimmer_both_ends() {
        let trimmer = SilenceTrimmer::with_settings(0.01, 0.0);
        let mut samples = vec![0.0; 10]; // 5 silent frames
        samples.extend_from_slice(&[0.5, -0.5, 0.3, -0.3]); // 2 sound frames
        samples.extend_from_slice(&[0.0; 10]); // 5 silent frames
        let mut buf = AudioBuffer::new(samples);
        trimmer.process(&mut buf).unwrap();
        assert_eq!(buf.frame_count(), 2);
        assert_eq!(buf.left(0), 0.5);
    }

    #[test]
    fn test_silence_trimmer_with_padding() {
        let trimmer = SilenceTrimmer::with_settings(0.01, 0.005);
        let padding_frames = (0.005 * SAMPLE_RATE as f32) as usize;

        // Many frames of silence, then sound, then more silence
        let silent_frames = padding_frames + 50;
        let mut samples = vec![0.0; silent_frames * 2]; // leading silence
        samples.extend_from_slice(&[0.5, -0.5]); // 1 sound frame
        samples.extend_from_slice(&vec![0.0; silent_frames * 2]); // trailing silence
        let mut buf = AudioBuffer::new(samples);
        trimmer.process(&mut buf).unwrap();

        // Should have sound frame + padding on each side
        assert!(buf.frame_count() >= 1); // at least the sound frame
        assert!(buf.frame_count() <= 1 + padding_frames * 2 + 1); // sound + padding both sides
    }

    #[test]
    fn test_silence_trimmer_threshold() {
        // Samples at exactly threshold should be considered silence
        let trimmer = SilenceTrimmer::with_settings(0.1, 0.0);
        let mut buf = AudioBuffer::new(vec![
            0.05, 0.05, // below threshold
            0.5, -0.5, // above threshold
            0.05, 0.05, // below threshold
        ]);
        trimmer.process(&mut buf).unwrap();
        assert_eq!(buf.frame_count(), 1);
        assert_eq!(buf.left(0), 0.5);
    }

    // --- VolumeAdjust tests ---

    #[test]
    fn test_volume_adjust_unity() {
        let vol = VolumeAdjust::new(1.0);
        let mut buf = AudioBuffer::new(vec![0.5, -0.5, 0.3, -0.3]);
        vol.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.5, -0.5, 0.3, -0.3]);
    }

    #[test]
    fn test_volume_adjust_half() {
        let vol = VolumeAdjust::new(0.5);
        let mut buf = AudioBuffer::new(vec![0.8, -0.8, 0.4, -0.4]);
        vol.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.4, -0.4, 0.2, -0.2]);
    }

    #[test]
    fn test_volume_adjust_double() {
        let vol = VolumeAdjust::new(2.0);
        let mut buf = AudioBuffer::new(vec![0.3, -0.3]);
        vol.process(&mut buf).unwrap();
        assert!((buf.samples[0] - 0.6).abs() < 1e-6);
        assert!((buf.samples[1] - (-0.6)).abs() < 1e-6);
    }

    #[test]
    fn test_volume_adjust_clamp() {
        let vol = VolumeAdjust::new(3.0);
        let mut buf = AudioBuffer::new(vec![0.5, -0.5]);
        vol.process(&mut buf).unwrap();
        // 0.5 * 3.0 = 1.5, clamped to 1.0
        assert_eq!(buf.samples[0], 1.0);
        assert_eq!(buf.samples[1], -1.0);
    }

    #[test]
    fn test_volume_adjust_zero() {
        let vol = VolumeAdjust::new(0.0);
        let mut buf = AudioBuffer::new(vec![0.5, -0.5, 1.0, -1.0]);
        vol.process(&mut buf).unwrap();
        assert!(buf.samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_volume_adjust_empty() {
        let vol = VolumeAdjust::new(2.0);
        let mut buf = AudioBuffer::empty();
        vol.process(&mut buf).unwrap();
        assert!(buf.is_empty());
    }

    // --- ChannelRouter tests ---

    #[test]
    fn test_channel_router_both() {
        let router = ChannelRouter::new(ChannelMode::Both);
        let mut buf = AudioBuffer::new(vec![0.5, -0.3, 0.7, -0.1]);
        router.process(&mut buf).unwrap();
        assert_eq!(buf.samples, vec![0.5, -0.3, 0.7, -0.1]);
    }

    #[test]
    fn test_channel_router_left() {
        let router = ChannelRouter::new(ChannelMode::Left);
        let mut buf = AudioBuffer::new(vec![0.5, -0.3, 0.7, -0.1]);
        router.process(&mut buf).unwrap();
        // Right channel zeroed
        assert_eq!(buf.left(0), 0.5);
        assert_eq!(buf.right(0), 0.0);
        assert_eq!(buf.left(1), 0.7);
        assert_eq!(buf.right(1), 0.0);
    }

    #[test]
    fn test_channel_router_right() {
        let router = ChannelRouter::new(ChannelMode::Right);
        let mut buf = AudioBuffer::new(vec![0.5, -0.3, 0.7, -0.1]);
        router.process(&mut buf).unwrap();
        // Left channel zeroed
        assert_eq!(buf.left(0), 0.0);
        assert_eq!(buf.right(0), -0.3);
        assert_eq!(buf.left(1), 0.0);
        assert_eq!(buf.right(1), -0.1);
    }

    #[test]
    fn test_channel_router_empty() {
        let router = ChannelRouter::new(ChannelMode::Left);
        let mut buf = AudioBuffer::empty();
        router.process(&mut buf).unwrap();
        assert!(buf.is_empty());
    }

    // --- Pipeline integration tests ---

    #[test]
    fn test_pipeline_volume_then_router() {
        use crate::pipeline::AudioPipeline;

        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(VolumeAdjust::new(0.5)));
        pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Left)));

        let mut buf = AudioBuffer::new(vec![0.8, -0.8, 0.4, -0.4]);
        pipeline.process(&mut buf).unwrap();

        // Volume first: [0.4, -0.4, 0.2, -0.2]
        // Then left routing: right channel zeroed
        assert!((buf.left(0) - 0.4).abs() < 1e-6);
        assert_eq!(buf.right(0), 0.0);
        assert!((buf.left(1) - 0.2).abs() < 1e-6);
        assert_eq!(buf.right(1), 0.0);
    }

    #[test]
    fn test_pipeline_trimmer_then_volume() {
        use crate::pipeline::AudioPipeline;

        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));
        pipeline.push(Box::new(VolumeAdjust::new(2.0)));

        // Leading silence + sound
        let mut samples = vec![0.0; 20]; // 10 silent frames
        samples.extend_from_slice(&[0.3, -0.3]); // 1 sound frame
        let mut buf = AudioBuffer::new(samples);
        pipeline.process(&mut buf).unwrap();

        // After trim: [0.3, -0.3]
        // After volume: [0.6, -0.6]
        assert_eq!(buf.frame_count(), 1);
        assert!((buf.left(0) - 0.6).abs() < 1e-6);
        assert!((buf.right(0) - (-0.6)).abs() < 1e-6);
    }

    #[test]
    fn test_full_pipeline() {
        use crate::pipeline::AudioPipeline;

        let mut pipeline = AudioPipeline::new();
        pipeline.push(Box::new(SilenceTrimmer::with_settings(0.01, 0.0)));
        pipeline.push(Box::new(VolumeAdjust::new(0.5)));
        pipeline.push(Box::new(ChannelRouter::new(ChannelMode::Right)));

        let mut samples = vec![0.0; 10]; // leading silence
        samples.extend_from_slice(&[0.8, -0.6, 0.4, -0.2]);
        samples.extend_from_slice(&[0.0; 10]); // trailing silence

        let mut buf = AudioBuffer::new(samples);
        pipeline.process(&mut buf).unwrap();

        // After trim: [0.8, -0.6, 0.4, -0.2] (2 frames)
        // After volume *0.5: [0.4, -0.3, 0.2, -0.1]
        // After right routing: left zeroed
        assert_eq!(buf.frame_count(), 2);
        assert_eq!(buf.left(0), 0.0);
        assert!((buf.right(0) - (-0.3)).abs() < 1e-6);
        assert_eq!(buf.left(1), 0.0);
        assert!((buf.right(1) - (-0.1)).abs() < 1e-6);
    }
}
