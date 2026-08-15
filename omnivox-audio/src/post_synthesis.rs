//! Stateful, duration-preserving post-synthesis processing.
//!
//! Parameters ramp across chunk and voice boundaries. Filter and delay state
//! persists between bounded render windows; reverb and echo return an explicit
//! final tail rather than shifting word or semantic-event positions.

use crate::buffer::{AudioBuffer, SAMPLE_RATE};

pub const EFFECT_BOUNDARY_RAMP_FRAMES: usize = SAMPLE_RATE as usize / 200; // 5 ms
pub const MAX_EFFECT_TAIL_FRAMES: usize = SAMPLE_RATE as usize * 4;
const QUIET_TAIL_FRAMES: usize = SAMPLE_RATE as usize / 50; // 20 ms
const QUIET_THRESHOLD: f32 = 0.0001;
const ECHO_DELAY_FRAMES: usize = SAMPLE_RATE as usize * 180 / 1000;
const REVERB_DELAYS: [usize; 4] = [1309, 1637, 1811, 1931];

/// Concrete DSP parameters. Filters use Hz, pan uses -1.0..=1.0, and wet
/// effects use 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSynthesisParameters {
    pub gain: f32,
    pub low_pass_hz: Option<f32>,
    pub high_pass_hz: Option<f32>,
    pub pan: f32,
    pub reverb: f32,
    pub echo: f32,
}

impl Default for PostSynthesisParameters {
    fn default() -> Self {
        Self {
            gain: 1.0,
            low_pass_hz: None,
            high_pass_hz: None,
            pan: 0.0,
            reverb: 0.0,
            echo: 0.0,
        }
    }
}

impl PostSynthesisParameters {
    pub fn sanitized(mut self) -> Self {
        self.gain = finite_or(self.gain, 1.0).clamp(0.0, 4.0);
        self.low_pass_hz = self
            .low_pass_hz
            .map(|value| finite_or(value, 20_000.0).clamp(80.0, 20_000.0));
        self.high_pass_hz = self
            .high_pass_hz
            .map(|value| finite_or(value, 20.0).clamp(20.0, 8_000.0));
        self.pan = finite_or(self.pan, 0.0).clamp(-1.0, 1.0);
        self.reverb = finite_or(self.reverb, 0.0).clamp(0.0, 1.0);
        self.echo = finite_or(self.echo, 0.0).clamp(0.0, 1.0);
        self
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[derive(Debug, Clone)]
pub struct ProcessedEffectWindow {
    pub audio: AudioBuffer,
    pub tail: Option<AudioBuffer>,
}

#[derive(Debug, Clone, Copy)]
struct EffectiveParameters {
    gain: f32,
    low_pass_hz: f32,
    low_pass_mix: f32,
    high_pass_hz: f32,
    high_pass_mix: f32,
    pan: f32,
    reverb: f32,
    echo: f32,
}

impl From<PostSynthesisParameters> for EffectiveParameters {
    fn from(value: PostSynthesisParameters) -> Self {
        let value = value.sanitized();
        Self {
            gain: value.gain,
            low_pass_hz: value.low_pass_hz.unwrap_or(20_000.0),
            low_pass_mix: value.low_pass_hz.is_some() as u8 as f32,
            high_pass_hz: value.high_pass_hz.unwrap_or(20.0),
            high_pass_mix: value.high_pass_hz.is_some() as u8 as f32,
            pan: value.pan,
            reverb: value.reverb,
            echo: value.echo,
        }
    }
}

impl EffectiveParameters {
    fn interpolate(self, target: Self, amount: f32) -> Self {
        let blend = |from: f32, to: f32| from + (to - from) * amount;
        Self {
            gain: blend(self.gain, target.gain),
            low_pass_hz: blend(self.low_pass_hz, target.low_pass_hz),
            low_pass_mix: blend(self.low_pass_mix, target.low_pass_mix),
            high_pass_hz: blend(self.high_pass_hz, target.high_pass_hz),
            high_pass_mix: blend(self.high_pass_mix, target.high_pass_mix),
            pan: blend(self.pan, target.pan),
            reverb: blend(self.reverb, target.reverb),
            echo: blend(self.echo, target.echo),
        }
    }
}

/// Stateful processor shared by consecutive synthesis windows in one
/// presentation.
pub struct PostSynthesisProcessor {
    current: EffectiveParameters,
    low_state: [f32; 2],
    high_input: [f32; 2],
    high_output: [f32; 2],
    echo: Vec<[f32; 2]>,
    echo_position: usize,
    reverb: Vec<Vec<[f32; 2]>>,
    reverb_positions: [usize; 4],
    tail_active: bool,
}

impl Default for PostSynthesisProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl PostSynthesisProcessor {
    pub fn new() -> Self {
        Self {
            current: PostSynthesisParameters::default().into(),
            low_state: [0.0; 2],
            high_input: [0.0; 2],
            high_output: [0.0; 2],
            echo: vec![[0.0; 2]; ECHO_DELAY_FRAMES],
            echo_position: 0,
            reverb: REVERB_DELAYS
                .iter()
                .map(|delay| vec![[0.0; 2]; *delay])
                .collect(),
            reverb_positions: [0; 4],
            tail_active: false,
        }
    }

    pub fn has_tail(&self) -> bool {
        self.tail_active
    }

    /// Process one canonical window. The primary duration never changes.
    pub fn process_window(
        &mut self,
        input: &AudioBuffer,
        target: PostSynthesisParameters,
        final_window: bool,
    ) -> ProcessedEffectWindow {
        let target = EffectiveParameters::from(target);
        let from = self.current;
        let ramp_frames = input.frame_count().min(EFFECT_BOUNDARY_RAMP_FRAMES);
        let mut samples = Vec::with_capacity(input.samples.len());
        for (frame_index, frame) in input.samples.chunks_exact(2).enumerate() {
            let amount = if frame_index < ramp_frames && ramp_frames > 0 {
                (frame_index + 1) as f32 / ramp_frames as f32
            } else {
                1.0
            };
            let parameters = from.interpolate(target, amount);
            let output = self.process_frame([frame[0], frame[1]], parameters);
            samples.extend_from_slice(&output);
        }
        self.current = target;

        let tail = if final_window && self.tail_active {
            let tail = self.flush_tail(target);
            (!tail.is_empty()).then(|| AudioBuffer::new(tail))
        } else {
            None
        };
        ProcessedEffectWindow {
            audio: AudioBuffer::new(samples),
            tail,
        }
    }

    pub fn finish(&mut self) -> Option<AudioBuffer> {
        if !self.tail_active {
            return None;
        }
        let tail = self.flush_tail(self.current);
        (!tail.is_empty()).then(|| AudioBuffer::new(tail))
    }

    fn process_frame(&mut self, mut frame: [f32; 2], parameters: EffectiveParameters) -> [f32; 2] {
        let source_audible = frame.iter().any(|sample| sample.abs() > QUIET_THRESHOLD);
        if source_audible && (parameters.echo > 0.0 || parameters.reverb > 0.0) {
            self.tail_active = true;
        }

        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample *= parameters.gain;
            let low_alpha = 1.0
                - (-2.0 * std::f32::consts::PI * parameters.low_pass_hz / SAMPLE_RATE as f32).exp();
            self.low_state[channel] += low_alpha * (*sample - self.low_state[channel]);
            *sample = *sample * (1.0 - parameters.low_pass_mix)
                + self.low_state[channel] * parameters.low_pass_mix;

            let dt = 1.0 / SAMPLE_RATE as f32;
            let rc = 1.0 / (2.0 * std::f32::consts::PI * parameters.high_pass_hz);
            let high_alpha = rc / (rc + dt);
            let high =
                high_alpha * (self.high_output[channel] + *sample - self.high_input[channel]);
            self.high_input[channel] = *sample;
            self.high_output[channel] = high;
            *sample = *sample * (1.0 - parameters.high_pass_mix) + high * parameters.high_pass_mix;
        }

        if parameters.pan < 0.0 {
            frame[1] *= 1.0 + parameters.pan;
        } else {
            frame[0] *= 1.0 - parameters.pan;
        }

        let delayed = self.echo[self.echo_position];
        let echo_feedback = parameters.echo * 0.72;
        let echo_mix = parameters.echo * 0.7;
        self.echo[self.echo_position] = if parameters.echo > 0.0 {
            [
                frame[0] + delayed[0] * echo_feedback,
                frame[1] + delayed[1] * echo_feedback,
            ]
        } else {
            [0.0; 2]
        };
        self.echo_position = (self.echo_position + 1) % self.echo.len();
        frame[0] += delayed[0] * echo_mix;
        frame[1] += delayed[1] * echo_mix;

        let mut reverberated = [0.0_f32; 2];
        for index in 0..self.reverb.len() {
            let position = self.reverb_positions[index];
            let delayed = self.reverb[index][position];
            reverberated[0] += delayed[0];
            reverberated[1] += delayed[1];
            let feedback = parameters.reverb * (0.68 + index as f32 * 0.035);
            self.reverb[index][position] = if parameters.reverb > 0.0 {
                [
                    frame[0] + delayed[0] * feedback,
                    frame[1] + delayed[1] * feedback,
                ]
            } else {
                [0.0; 2]
            };
            self.reverb_positions[index] = (position + 1) % self.reverb[index].len();
        }
        frame[0] += reverberated[0] * parameters.reverb * 0.15;
        frame[1] += reverberated[1] * parameters.reverb * 0.15;
        [frame[0].clamp(-1.0, 1.0), frame[1].clamp(-1.0, 1.0)]
    }

    fn flush_tail(&mut self, parameters: EffectiveParameters) -> Vec<f32> {
        let minimum_frames = ECHO_DELAY_FRAMES.max(*REVERB_DELAYS.iter().max().unwrap());
        let mut quiet_frames = 0;
        let mut tail = Vec::new();
        for frame_index in 0..MAX_EFFECT_TAIL_FRAMES {
            let frame = self.process_frame([0.0, 0.0], parameters);
            tail.extend_from_slice(&frame);
            if frame.iter().all(|sample| sample.abs() <= QUIET_THRESHOLD) {
                quiet_frames += 1;
            } else {
                quiet_frames = 0;
            }
            if frame_index >= minimum_frames && quiet_frames >= QUIET_TAIL_FRAMES {
                break;
            }
        }
        self.tail_active = false;
        if let Some(last_audible_sample) = tail
            .iter()
            .rposition(|sample| sample.abs() > QUIET_THRESHOLD)
        {
            let retained_samples = ((last_audible_sample / 2) + 1) * 2;
            tail.truncate(retained_samples);
        } else {
            tail.clear();
        }
        tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_processing_is_duration_preserving() {
        let input = AudioBuffer::new(vec![0.25, -0.25, 0.5, -0.5]);
        let mut processor = PostSynthesisProcessor::new();

        let processed = processor.process_window(&input, PostSynthesisParameters::default(), true);

        assert_eq!(processed.audio.samples, input.samples);
        assert!(processed.tail.is_none());
    }

    #[test]
    fn gain_and_pan_ramp_without_changing_duration() {
        let input = AudioBuffer::new(vec![0.25; (EFFECT_BOUNDARY_RAMP_FRAMES + 10) * 2]);
        let mut processor = PostSynthesisProcessor::new();
        let processed = processor.process_window(
            &input,
            PostSynthesisParameters {
                gain: 2.0,
                pan: 1.0,
                ..PostSynthesisParameters::default()
            },
            true,
        );

        assert_eq!(processed.audio.frame_count(), input.frame_count());
        assert!(processed.audio.left(0) > 0.0);
        assert!(processed.audio.left(EFFECT_BOUNDARY_RAMP_FRAMES + 1).abs() < 0.0001);
        assert!(processed.audio.right(EFFECT_BOUNDARY_RAMP_FRAMES + 1) > 0.49);
    }

    #[test]
    fn echo_state_crosses_windows_and_returns_a_bounded_final_tail() {
        let mut first_samples = vec![0.0; (ECHO_DELAY_FRAMES - 1) * 2];
        first_samples[0] = 0.5;
        first_samples[1] = 0.5;
        let first = AudioBuffer::new(first_samples);
        let second = AudioBuffer::new(vec![0.0; 4]);
        let parameters = PostSynthesisParameters {
            echo: 0.8,
            ..PostSynthesisParameters::default()
        };
        let mut processor = PostSynthesisProcessor::new();

        let first = processor.process_window(&first, parameters, false);
        let second = processor.process_window(&second, parameters, true);

        assert!(first.tail.is_none());
        assert!(second
            .audio
            .samples
            .iter()
            .any(|sample| sample.abs() > 0.01));
        let tail = second.tail.unwrap();
        assert!(!tail.is_empty());
        assert!(tail.frame_count() <= MAX_EFFECT_TAIL_FRAMES);
        assert!(!processor.has_tail());
    }

    #[test]
    fn reverb_produces_an_explicit_tail_without_extending_primary_audio() {
        let input = AudioBuffer::new(vec![0.5, 0.5]);
        let mut processor = PostSynthesisProcessor::new();
        let processed = processor.process_window(
            &input,
            PostSynthesisParameters {
                reverb: 0.7,
                ..PostSynthesisParameters::default()
            },
            true,
        );

        assert_eq!(processed.audio.frame_count(), 1);
        assert!(processed.tail.unwrap().frame_count() <= MAX_EFFECT_TAIL_FRAMES);
    }

    #[test]
    fn enabling_a_delay_does_not_replay_earlier_dry_audio() {
        let dry = AudioBuffer::new(vec![0.5; (ECHO_DELAY_FRAMES + 4) * 2]);
        let silence = AudioBuffer::new(vec![0.0; (ECHO_DELAY_FRAMES + 4) * 2]);
        let mut processor = PostSynthesisProcessor::new();

        processor.process_window(&dry, PostSynthesisParameters::default(), false);
        let processed = processor.process_window(
            &silence,
            PostSynthesisParameters {
                echo: 0.8,
                reverb: 0.8,
                ..PostSynthesisParameters::default()
            },
            true,
        );

        assert!(processed
            .audio
            .samples
            .iter()
            .all(|sample| sample.abs() < 0.0001));
        assert!(processed.tail.is_none());
    }
}
