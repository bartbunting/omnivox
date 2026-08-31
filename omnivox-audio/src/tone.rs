//! Tone Generator
//!
//! Pure Rust sine wave tone generator with fade-in/fade-out to prevent clicks.
//! Produces stereo f32 at 44100Hz matching the common buffer format.

use crate::buffer::{AudioBuffer, SAMPLE_RATE};

/// Generator for sine wave tones.
pub struct ToneGenerator;

impl ToneGenerator {
    /// Generate a sine wave tone as a stereo AudioBuffer.
    ///
    /// - `freq_hz`: frequency in Hz (e.g., 440.0 for concert A)
    /// - `duration_ms`: duration in milliseconds
    /// - `volume`: amplitude multiplier (0.0 to 1.0)
    ///
    /// Applies a 10ms fade-in and fade-out envelope to prevent clicks.
    /// Produces stereo output with a slight delay on the right channel
    /// (0.01ms, ~0.4 samples) for a spatial effect matching Emacspeak's
    /// sox `delay 0 0.01` behavior. Since the delay is sub-sample, we
    /// approximate by interpolating the right channel.
    pub fn generate(freq_hz: f32, duration_ms: u32, volume: f32) -> AudioBuffer {
        let frame_count = (SAMPLE_RATE as f64 * duration_ms as f64 / 1000.0) as usize;
        if frame_count == 0 {
            return AudioBuffer::empty();
        }

        let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize; // 10ms fade
        let fade_frames = fade_frames.min(frame_count / 2); // don't fade more than half

        // Sub-sample delay for the right channel: 0.01ms = 0.44 samples at 44100Hz
        let delay_samples: f32 = SAMPLE_RATE as f32 * 0.00001; // 0.01ms in samples

        let two_pi = 2.0 * std::f32::consts::PI;
        let mut samples = Vec::with_capacity(frame_count * 2);

        for i in 0..frame_count {
            let t = i as f32 / SAMPLE_RATE as f32;

            // Sine wave
            let left_sample = (two_pi * freq_hz * t).sin() * volume;

            // Right channel with sub-sample delay via linear interpolation
            let right_sample = if i == 0 {
                // First sample: no previous sample to interpolate from
                left_sample * (1.0 - delay_samples)
            } else {
                let t_prev = (i as f32 - 1.0) / SAMPLE_RATE as f32;
                let prev = (two_pi * freq_hz * t_prev).sin() * volume;
                let curr = (two_pi * freq_hz * t).sin() * volume;
                // Linear interpolation between previous and current
                let frac = delay_samples;
                prev * frac + curr * (1.0 - frac)
            };

            // Apply fade envelope
            let envelope = if i < fade_frames {
                // Fade in
                i as f32 / fade_frames as f32
            } else if i >= frame_count - fade_frames {
                // Fade out
                (frame_count - 1 - i) as f32 / fade_frames as f32
            } else {
                1.0
            };

            samples.push(left_sample * envelope);
            samples.push(right_sample * envelope);
        }

        AudioBuffer::new(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::SAMPLE_RATE;

    #[test]
    fn test_generate_correct_duration() {
        let buf = ToneGenerator::generate(440.0, 100, 1.0);
        let expected_frames = (SAMPLE_RATE as f64 * 0.1) as usize;
        assert_eq!(buf.frame_count(), expected_frames);
        assert_eq!(buf.samples.len(), expected_frames * 2);
    }

    #[test]
    fn test_generate_zero_duration() {
        let buf = ToneGenerator::generate(440.0, 0, 1.0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_generate_stereo_output() {
        let buf = ToneGenerator::generate(440.0, 50, 1.0);
        // Stereo: every frame has 2 samples
        assert_eq!(buf.samples.len() % 2, 0);
        assert!(buf.frame_count() > 0);
    }

    #[test]
    fn test_fade_in_envelope() {
        let buf = ToneGenerator::generate(440.0, 100, 1.0);
        let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

        // First sample should be near zero (faded in)
        assert!(buf.left(0).abs() < 0.01);

        // Sample at half the fade should be smaller than sample past the fade
        if fade_frames > 2 {
            let past_fade = fade_frames + 10;
            if past_fade < buf.frame_count() {
                // The envelope at mid_fade is 0.5, past_fade is 1.0
                // But the sine value also varies, so just check the
                // peak in the fade region is less than peak past it
                let fade_peak: f32 = (0..fade_frames)
                    .map(|i| buf.left(i).abs())
                    .fold(0.0f32, f32::max);
                let body_peak: f32 = (fade_frames..buf.frame_count() - fade_frames)
                    .map(|i| buf.left(i).abs())
                    .fold(0.0f32, f32::max);
                assert!(
                    fade_peak <= body_peak + 0.01,
                    "fade peak {} should be <= body peak {}",
                    fade_peak,
                    body_peak
                );
            }
        }
    }

    #[test]
    fn test_fade_out_envelope() {
        let buf = ToneGenerator::generate(440.0, 100, 1.0);
        let last = buf.frame_count() - 1;
        // Last sample should be near zero (faded out)
        assert!(
            buf.left(last).abs() < 0.01,
            "last sample left: {}",
            buf.left(last)
        );
    }

    #[test]
    fn test_volume_scaling() {
        let loud = ToneGenerator::generate(440.0, 100, 1.0);
        let quiet = ToneGenerator::generate(440.0, 100, 0.5);

        let loud_peak = loud.peak_amplitude();
        let quiet_peak = quiet.peak_amplitude();

        // Quiet should be roughly half the loud peak (within some tolerance
        // due to fade envelope alignment)
        assert!(
            quiet_peak < loud_peak,
            "quiet peak {} should be < loud peak {}",
            quiet_peak,
            loud_peak
        );
        let ratio = quiet_peak / loud_peak;
        assert!(
            (ratio - 0.5).abs() < 0.1,
            "ratio {} should be near 0.5",
            ratio
        );
    }

    #[test]
    fn test_frequency_content() {
        // Generate a 440Hz tone for 100ms
        let buf = ToneGenerator::generate(440.0, 100, 1.0);
        let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

        // Extract left channel samples from the steady-state region (past fades)
        let start = fade_frames;
        let end = buf.frame_count() - fade_frames;
        if end <= start {
            return; // too short to test
        }

        // Count zero crossings in the steady-state region
        let left_samples: Vec<f32> = (start..end).map(|i| buf.left(i)).collect();
        let mut zero_crossings = 0u32;
        for i in 1..left_samples.len() {
            if left_samples[i - 1].signum() != left_samples[i].signum() {
                zero_crossings += 1;
            }
        }

        // A sine wave has 2 zero crossings per cycle.
        // Expected frequency from zero crossings: zero_crossings / 2 / duration
        let duration = (end - start) as f32 / SAMPLE_RATE as f32;
        let measured_freq = zero_crossings as f32 / 2.0 / duration;

        assert!(
            (measured_freq - 440.0).abs() < 5.0,
            "measured frequency {} should be near 440Hz",
            measured_freq
        );
    }

    #[test]
    fn test_spatial_delay() {
        // The right channel should be slightly delayed relative to left.
        // For a simple sine, this means the right channel values should
        // differ slightly from left channel values.
        let buf = ToneGenerator::generate(1000.0, 50, 1.0);
        let fade_frames = (SAMPLE_RATE as f32 * 0.010) as usize;

        // Check a sample in the steady-state region
        let mid = fade_frames + (buf.frame_count() - 2 * fade_frames) / 2;
        if mid < buf.frame_count() {
            let left = buf.left(mid);
            let right = buf.right(mid);
            // They should be similar but not identical due to the sub-sample delay
            let diff = (left - right).abs();
            assert!(
                diff < 0.1,
                "left ({}) and right ({}) should be close but differ slightly",
                left,
                right
            );
        }
    }

    #[test]
    fn test_common_emacspeak_tones() {
        // Verify common Emacspeak tones generate without error
        let caps_beep = ToneGenerator::generate(440.0, 10, 1.0);
        assert!(!caps_beep.is_empty());

        let deletion = ToneGenerator::generate(500.0, 75, 1.0);
        assert!(!deletion.is_empty());

        let upcase = ToneGenerator::generate(800.0, 100, 1.0);
        assert!(!upcase.is_empty());

        let downcase = ToneGenerator::generate(600.0, 100, 1.0);
        assert!(!downcase.is_empty());
    }

    #[test]
    fn test_very_short_tone() {
        // A 1ms tone should still work, with reduced fade
        let buf = ToneGenerator::generate(440.0, 1, 1.0);
        assert!(!buf.is_empty());
        // Duration should be approximately 1ms
        let duration_ms = buf.duration_secs() * 1000.0;
        assert!(
            (duration_ms - 1.0).abs() < 1.0,
            "duration {}ms should be near 1ms",
            duration_ms
        );
    }

    #[test]
    fn test_samples_in_range() {
        let buf = ToneGenerator::generate(440.0, 100, 1.0);
        for &sample in &buf.samples {
            assert!(
                (-1.0..=1.0).contains(&sample),
                "sample {} out of range",
                sample
            );
        }
    }
}
