//! Audio Effects
//!
//! Stateless buffer-wide effects implementing the AudioEffect trait. Stateful
//! presentation effects such as chorus, echo, and reverb live in the bounded
//! post-synthesis processor.

use crate::buffer::{AudioBuffer, SAMPLE_RATE};
use crate::pipeline::AudioEffect;
use crate::AudioError;
use omnivox_core::ChannelMode;
use std::collections::VecDeque;

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

/// Stateful silence trimming for canonical PCM delivered in bounded windows.
///
/// Leading silence is reduced as soon as the first audible frame arrives.
/// Silence after audible PCM is retained until later sound proves it internal,
/// or [`Self::finish`] proves it trailing. This preserves the buffered
/// trimmer's exact samples across arbitrary input window boundaries.
pub struct ProgressiveSilenceTrimmer {
    threshold: f32,
    leading_padding_frames: usize,
    trailing_padding_frames: usize,
    leading: VecDeque<[f32; 2]>,
    pending_silence: Vec<f32>,
    input_frames: usize,
    output_frames: usize,
    leading_silence_frames: usize,
    removed_leading_frames: Option<usize>,
    audible: bool,
    finished: bool,
}

impl ProgressiveSilenceTrimmer {
    /// Create an incremental trimmer with the same settings as
    /// [`SilenceTrimmer::with_asymmetric_padding`].
    pub fn with_asymmetric_padding(
        threshold: f32,
        leading_padding_secs: f32,
        trailing_padding_secs: f32,
    ) -> Self {
        Self {
            threshold,
            leading_padding_frames: (leading_padding_secs * SAMPLE_RATE as f32) as usize,
            trailing_padding_frames: (trailing_padding_secs * SAMPLE_RATE as f32) as usize,
            leading: VecDeque::new(),
            pending_silence: Vec::new(),
            input_frames: 0,
            output_frames: 0,
            leading_silence_frames: 0,
            removed_leading_frames: None,
            audible: false,
            finished: false,
        }
    }

    /// Process one canonical PCM window, returning any frames now known to be
    /// part of the trimmed result.
    pub fn process_window(&mut self, input: AudioBuffer) -> Result<AudioBuffer, AudioError> {
        if self.finished {
            return Err(AudioError::EffectError(
                "progressive silence trimmer is already finished".to_owned(),
            ));
        }
        let mut output = Vec::with_capacity(input.samples.len());
        for frame in input.samples.chunks_exact(2) {
            self.input_frames = self.input_frames.saturating_add(1);
            let frame = [frame[0], frame[1]];
            let audible = frame[0].abs() > self.threshold || frame[1].abs() > self.threshold;
            if !self.audible {
                if !audible {
                    self.leading_silence_frames = self.leading_silence_frames.saturating_add(1);
                    self.leading.push_back(frame);
                    if self.leading.len() > self.leading_padding_frames {
                        self.leading.pop_front();
                    }
                    continue;
                }
                self.audible = true;
                self.removed_leading_frames = Some(
                    self.leading_silence_frames
                        .saturating_sub(self.leading.len()),
                );
                while let Some(frame) = self.leading.pop_front() {
                    output.extend_from_slice(&frame);
                }
                output.extend_from_slice(&frame);
                continue;
            }

            if audible {
                output.append(&mut self.pending_silence);
                output.extend_from_slice(&frame);
            } else {
                self.pending_silence.extend_from_slice(&frame);
            }
        }
        self.output_frames = self.output_frames.saturating_add(output.len() / 2);
        Ok(AudioBuffer::new(output))
    }

    /// Return final retained trailing padding and the complete trim report.
    pub fn finish(&mut self) -> Result<(AudioBuffer, SilenceTrimReport), AudioError> {
        if self.finished {
            return Err(AudioError::EffectError(
                "progressive silence trimmer is already finished".to_owned(),
            ));
        }
        self.finished = true;
        if !self.audible {
            self.leading.clear();
            self.pending_silence.clear();
            self.removed_leading_frames = Some(self.input_frames);
            return Ok((
                AudioBuffer::empty(),
                SilenceTrimReport {
                    input_frames: self.input_frames,
                    output_frames: 0,
                    removed_leading_frames: self.input_frames,
                    removed_trailing_frames: 0,
                },
            ));
        }

        let pending_frames = self.pending_silence.len() / 2;
        let retained_frames = pending_frames.min(self.trailing_padding_frames);
        let retained_samples = retained_frames * 2;
        let start = self.pending_silence.len().saturating_sub(retained_samples);
        let tail = AudioBuffer::new(self.pending_silence.split_off(start));
        self.pending_silence.clear();
        self.output_frames = self.output_frames.saturating_add(retained_frames);
        Ok((
            tail,
            SilenceTrimReport {
                input_frames: self.input_frames,
                output_frames: self.output_frames,
                removed_leading_frames: self.removed_leading_frames.unwrap_or(0),
                removed_trailing_frames: pending_frames.saturating_sub(retained_frames),
            },
        ))
    }

    /// Leading frames removed once the first audible frame has been observed.
    pub fn removed_leading_frames(&self) -> Option<usize> {
        self.removed_leading_frames
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
    fn progressive_trimmer_matches_buffered_samples_across_windows() {
        let leading_padding = 3.5 / SAMPLE_RATE as f32;
        let trailing_padding = 4.5 / SAMPLE_RATE as f32;
        let mut samples = vec![0.0; 10 * 2];
        samples.extend_from_slice(&[0.5, -0.5, 0.4, -0.4]);
        samples.extend_from_slice(&[0.0; 7 * 2]);
        samples.extend_from_slice(&[0.3, -0.3]);
        samples.extend_from_slice(&[0.0; 12 * 2]);

        let mut buffered = AudioBuffer::new(samples.clone());
        let expected_report =
            SilenceTrimmer::with_asymmetric_padding(0.01, leading_padding, trailing_padding)
                .process_with_report(&mut buffered)
                .unwrap();

        let mut progressive = ProgressiveSilenceTrimmer::with_asymmetric_padding(
            0.01,
            leading_padding,
            trailing_padding,
        );
        let mut actual = AudioBuffer::empty();
        let mut start = 0;
        for frames in [2, 9, 4, 16, 1] {
            let end = start + frames * 2;
            let output = progressive
                .process_window(AudioBuffer::new(samples[start..end].to_vec()))
                .unwrap();
            actual.append(&output);
            start = end;
        }
        let (tail, actual_report) = progressive.finish().unwrap();
        actual.append(&tail);

        assert_eq!(start, samples.len());
        assert_eq!(actual.samples, buffered.samples);
        assert_eq!(actual_report, expected_report);
    }

    #[test]
    fn progressive_trimmer_discards_an_all_silent_stream() {
        let mut progressive =
            ProgressiveSilenceTrimmer::with_asymmetric_padding(0.01, 0.005, 0.005);
        assert!(progressive
            .process_window(AudioBuffer::new(vec![0.0; 200]))
            .unwrap()
            .is_empty());
        assert_eq!(progressive.removed_leading_frames(), None);

        let (tail, report) = progressive.finish().unwrap();

        assert!(tail.is_empty());
        assert_eq!(report.input_frames, 100);
        assert_eq!(report.output_frames, 0);
        assert_eq!(report.removed_leading_frames, 100);
        assert_eq!(report.removed_trailing_frames, 0);
        assert_eq!(progressive.removed_leading_frames(), Some(100));
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
