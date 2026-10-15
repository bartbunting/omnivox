//! Bounded harmonic synthesis and fixed-duration envelopes.

use super::{check_cancelled, ToneGenerator};
use crate::{buffer::SAMPLE_RATE, AudioBuffer, AudioError};
use omnivox_core::tone::{AdsrEnvelope, ToneParameters, ToneShape, Waveform};
use std::f64::consts::{PI, TAU};

const MAX_HARMONICS: usize = 32;

impl ToneGenerator {
    /// Render one validated sound definition. Volume is per-cue gain only;
    /// callers apply master gain and routing once through the usual pipeline.
    /// Cancellation discards partial PCM and never returns a playable prefix.
    pub fn generate_shaped(
        tone: &ToneParameters,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<AudioBuffer, AudioError> {
        check_cancelled(cancelled)?;
        tone.validate().map_err(AudioError::InvalidFormat)?;
        if tone.shape == ToneShape::default() {
            return Self::generate_interruptible(
                tone.frequency_hz,
                tone.duration_ms,
                tone.volume,
                cancelled,
            );
        }
        let frames = SAMPLE_RATE as usize * tone.duration_ms as usize / 1000;
        let oscillator = Oscillator::new(tone.shape.waveform, tone.frequency_hz);
        let envelope = tone
            .shape
            .envelope
            .map(|adsr| Envelope::new(adsr, frames, tone.duration_ms));
        let fade = (SAMPLE_RATE as f32 * 0.010) as usize;
        let fade = fade.min(frames / 2);
        let delay = SAMPLE_RATE as f32 * 0.00001;
        check_cancelled(cancelled)?;
        let mut samples = Vec::with_capacity(frames * 2);
        let mut previous = 0.0;
        for frame in 0..frames {
            if frame != 0 && frame % 256 == 0 {
                check_cancelled(cancelled)?;
            }
            let left = oscillator.sample(frame) * tone.volume;
            let right = previous * delay + left * (1.0 - delay);
            previous = left;
            let level = envelope.as_ref().map_or_else(
                || {
                    if frame < fade {
                        frame as f32 / fade as f32
                    } else if frame >= frames - fade {
                        (frames - 1 - frame) as f32 / fade as f32
                    } else {
                        1.0
                    }
                },
                |envelope| envelope.level(frame),
            );
            samples.push(left * level);
            samples.push(right * level);
        }
        check_cancelled(cancelled)?;
        Ok(AudioBuffer::new(samples))
    }
}

struct Oscillator {
    frequency: f64,
    weights: [f64; MAX_HARMONICS],
    count: usize,
}

impl Oscillator {
    fn new(waveform: Waveform, frequency: f32) -> Self {
        let mut weights = [0.0; MAX_HARMONICS];
        // One fixed bound for every square frequency, not loudness matching.
        // The sum of absolute coefficients is at most one, including at
        // frequencies where only the fundamental fits below Nyquist.
        let square_bound: f64 = (0..MAX_HARMONICS).map(|k| 1.0 / (2 * k + 1) as f64).sum();
        let mut count = 0;
        for (k, weight) in weights.iter_mut().enumerate() {
            let harmonic = (2 * k + 1) as f64;
            if harmonic * frequency as f64 >= SAMPLE_RATE as f64 / 2.0 {
                break;
            }
            *weight = match waveform {
                Waveform::Sine if k == 0 => 1.0,
                Waveform::Sine => break,
                Waveform::Square => 1.0 / (square_bound * harmonic),
                Waveform::Triangle => {
                    let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                    sign * 8.0 / (PI * PI * harmonic * harmonic)
                }
            };
            count += 1;
        }
        Self {
            frequency: frequency as f64,
            weights,
            count,
        }
    }

    fn sample(&self, frame: usize) -> f32 {
        let phase = TAU * self.frequency * frame as f64 / SAMPLE_RATE as f64;
        let (mut sine, mut cosine) = phase.sin_cos();
        let (step_sine, step_cosine) = (2.0 * phase).sin_cos();
        let mut value = 0.0;
        for weight in &self.weights[..self.count] {
            value += weight * sine;
            (sine, cosine) = (
                sine * step_cosine + cosine * step_sine,
                cosine * step_cosine - sine * step_sine,
            );
        }
        value as f32
    }
}

struct Envelope {
    attack_end: usize,
    decay_end: usize,
    release_start: usize,
    last: usize,
    sustain: f32,
    cap: usize,
}

impl Envelope {
    fn new(adsr: AdsrEnvelope, frames: usize, duration_ms: u32) -> Self {
        let last = frames - 1;
        let attack = adsr.attack_ms as u64;
        let decay = adsr.decay_ms as u64;
        let release = adsr.release_ms as u64;
        let total = attack + decay + release;
        let denominator = total.max(duration_ms as u64);
        let boundary = |time| (time * last as u64 / denominator) as usize;
        let decay_end = boundary(attack + decay);
        Self {
            attack_end: boundary(attack),
            decay_end,
            release_start: if total >= duration_ms as u64 {
                decay_end
            } else {
                last - boundary(release)
            },
            last,
            sustain: adsr.sustain_level,
            cap: (SAMPLE_RATE as usize / 1000).min(last / 2),
        }
    }

    fn level(&self, frame: usize) -> f32 {
        let level = if frame < self.attack_end {
            frame as f32 / self.attack_end as f32
        } else if frame < self.decay_end {
            let fraction =
                (frame - self.attack_end) as f32 / (self.decay_end - self.attack_end) as f32;
            1.0 + (self.sustain - 1.0) * fraction
        } else if frame < self.release_start || self.release_start == self.last {
            self.sustain
        } else {
            self.sustain * (self.last - frame) as f32 / (self.last - self.release_start) as f32
        };
        let edge = frame.min(self.last - frame);
        let cap = if edge < self.cap {
            let x = edge as f32 / self.cap as f32;
            x * x * (3.0 - 2.0 * x)
        } else {
            1.0
        };
        level.min(cap)
    }
}

#[cfg(test)]
mod tests;
