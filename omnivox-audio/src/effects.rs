//! Audio Effects
//!
//! Stateless buffer-wide effects implementing the AudioEffect trait. Stateful
//! presentation effects such as chorus, echo, and reverb live in the bounded
//! post-synthesis processor.

use crate::buffer::{AudioBuffer, SAMPLE_RATE};
use crate::pipeline::AudioEffect;
use crate::AudioError;
use omnivox_core::ChannelMode;

/// Removes leading and trailing silence from an AudioBuffer.
///
/// Samples with absolute amplitude below the threshold are considered silent.
/// Separate leading/trailing padding allows fine control at chunk and voice
/// boundaries: interior segments within a dispatch batch use zero padding for
/// seamless joins, while batch edges preserve padding for natural spacing.
pub struct SilenceTrimmer {
    /// Amplitude threshold below which samples are considered silence
    pub threshold: f32,
    /// Padding to preserve before first audible frame, in seconds
    pub leading_padding_secs: f32,
    /// Padding to preserve after last audible frame, in seconds
    pub trailing_padding_secs: f32,
}

/// Frame changes made by a [`SilenceTrimmer`].
///
/// The counts always partition the input: leading frames, output frames, then
/// trailing frames. An entirely silent input is reported as wholly removed
/// leading silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilenceTrimReport {
    pub input_frames: usize,
    pub output_frames: usize,
    pub removed_leading_frames: usize,
    pub removed_trailing_frames: usize,
}

impl SilenceTrimReport {
    /// Map an input frame offset into the trimmed buffer.
    ///
    /// Offsets in removed leading or trailing silence are clamped to the
    /// corresponding output boundary.
    pub fn map_frame_offset(self, frame_offset: u64) -> u64 {
        frame_offset
            .saturating_sub(self.removed_leading_frames as u64)
            .min(self.output_frames as u64)
    }
}

impl SilenceTrimmer {
    /// Create a new SilenceTrimmer with default settings.
    ///
    /// Default threshold: 0.01, default padding: 5ms on both sides.
    pub fn new() -> Self {
        Self {
            threshold: 0.01,
            leading_padding_secs: 0.005,
            trailing_padding_secs: 0.005,
        }
    }

    /// Create a SilenceTrimmer with symmetric padding on both sides.
    pub fn with_settings(threshold: f32, padding_secs: f32) -> Self {
        Self {
            threshold,
            leading_padding_secs: padding_secs,
            trailing_padding_secs: padding_secs,
        }
    }

    /// Create a SilenceTrimmer with independent leading and trailing padding.
    pub fn with_asymmetric_padding(
        threshold: f32,
        leading_padding_secs: f32,
        trailing_padding_secs: f32,
    ) -> Self {
        Self {
            threshold,
            leading_padding_secs,
            trailing_padding_secs,
        }
    }

    /// Trim the buffer and report how its frame timeline changed.
    pub fn process_with_report(
        &self,
        buffer: &mut AudioBuffer,
    ) -> Result<SilenceTrimReport, AudioError> {
        let input_frames = buffer.frame_count();
        if buffer.is_empty() {
            return Ok(SilenceTrimReport {
                input_frames,
                output_frames: 0,
                removed_leading_frames: 0,
                removed_trailing_frames: 0,
            });
        }

        let leading_frames = (self.leading_padding_secs * SAMPLE_RATE as f32) as usize;
        let trailing_frames = (self.trailing_padding_secs * SAMPLE_RATE as f32) as usize;

        // Find first non-silent frame.
        let first_sound = (0..input_frames).find(|&i| {
            buffer.left(i).abs() > self.threshold || buffer.right(i).abs() > self.threshold
        });
        let Some(first_sound) = first_sound else {
            buffer.samples.clear();
            return Ok(SilenceTrimReport {
                input_frames,
                output_frames: 0,
                removed_leading_frames: input_frames,
                removed_trailing_frames: 0,
            });
        };

        // A first audible frame guarantees a last audible frame.
        let last_sound = (0..input_frames)
            .rev()
            .find(|&i| {
                buffer.left(i).abs() > self.threshold || buffer.right(i).abs() > self.threshold
            })
            .unwrap();

        // Apply padding without going beyond buffer bounds.
        let start_frame = first_sound.saturating_sub(leading_frames);
        let end_frame = last_sound
            .saturating_add(trailing_frames)
            .saturating_add(1)
            .min(input_frames);
        let output_frames = end_frame - start_frame;

        // Only trim if there's actually silence to remove.
        if start_frame > 0 || end_frame < input_frames {
            let start_sample = start_frame * 2;
            let end_sample = end_frame * 2;
            buffer.samples = buffer.samples[start_sample..end_sample].to_vec();
        }

        Ok(SilenceTrimReport {
            input_frames,
            output_frames,
            removed_leading_frames: start_frame,
            removed_trailing_frames: input_frames - end_frame,
        })
    }
}

impl Default for SilenceTrimmer {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for SilenceTrimmer {
    fn process(&self, buffer: &mut AudioBuffer) -> Result<(), AudioError> {
        self.process_with_report(buffer)?;
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
        let report = trimmer.process_with_report(&mut buf).unwrap();
        assert!(buf.is_empty());
        assert_eq!(
            report,
            SilenceTrimReport {
                input_frames: 0,
                output_frames: 0,
                removed_leading_frames: 0,
                removed_trailing_frames: 0,
            }
        );
    }

    #[test]
    fn test_silence_trimmer_all_silence() {
        let trimmer = SilenceTrimmer::new();
        let mut buf = AudioBuffer::new(vec![0.0; 1000]);
        let report = trimmer.process_with_report(&mut buf).unwrap();
        assert!(buf.is_empty());
        assert_eq!(report.input_frames, 500);
        assert_eq!(report.output_frames, 0);
        assert_eq!(report.removed_leading_frames, 500);
        assert_eq!(report.removed_trailing_frames, 0);
        assert_eq!(report.map_frame_offset(250), 0);
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
        let report = trimmer.process_with_report(&mut buf).unwrap();
        assert_eq!(buf.frame_count(), 2);
        assert_eq!(buf.left(0), 0.5);
        assert_eq!(report.input_frames, 12);
        assert_eq!(report.output_frames, 2);
        assert_eq!(report.removed_leading_frames, 5);
        assert_eq!(report.removed_trailing_frames, 5);
        assert_eq!(report.map_frame_offset(2), 0);
        assert_eq!(report.map_frame_offset(5), 0);
        assert_eq!(report.map_frame_offset(6), 1);
        assert_eq!(report.map_frame_offset(10), 2);
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
