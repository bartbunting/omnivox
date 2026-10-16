//! Bounded harmonic synthesis and fixed-duration envelopes.

use super::{check_cancelled, ToneGenerator};
use crate::{buffer::SAMPLE_RATE, AudioBuffer, AudioError};
use omnivox_core::tone::{AdsrEnvelope, EnvelopeCurve, ToneParameters, ToneShape, Waveform};
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
        let maximum_frequency = tone
            .frequency_hz
            .max(tone.shape.end_frequency_hz.unwrap_or(tone.frequency_hz));
        let oscillator = Oscillator::new(tone.shape.waveform, maximum_frequency);
        let extended = matches!(tone.shape.waveform, Waveform::Pulse | Waveform::Sawtooth)
            .then(|| ExtendedOscillator::new(tone.shape, maximum_frequency));
        let envelope = tone.shape.envelope.map(|adsr| {
            let mut envelope = Envelope::new(adsr, frames, tone.duration_ms);
            envelope.curve = tone.shape.envelope_curve.unwrap_or(EnvelopeCurve::Linear);
            envelope
        });
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
            let sample = if let Some(end) = tone.shape.end_frequency_hz {
                // Integral of a linear frequency ramp, in radians. Limiting
                // harmonics against both endpoints avoids aliases throughout.
                let time = frame as f64 / SAMPLE_RATE as f64;
                let duration = frames as f64 / SAMPLE_RATE as f64;
                let phase = TAU
                    * (tone.frequency_hz as f64 * time
                        + 0.5 * (end - tone.frequency_hz) as f64 * time * time / duration);
                extended
                    .as_ref()
                    .map_or_else(|| oscillator.sample_phase(phase), |o| o.sample(phase))
            } else if let Some(o) = &extended {
                o.sample(TAU * tone.frequency_hz as f64 * frame as f64 / SAMPLE_RATE as f64)
            } else {
                oscillator.sample(frame)
            };
            let left = sample * tone.volume;
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
                Waveform::Sine | Waveform::Pulse | Waveform::Sawtooth => break,
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
        self.sample_phase(phase)
    }

    fn sample_phase(&self, phase: f64) -> f32 {
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

// Full harmonic series for pulse and sawtooth. The frequency-independent
// absolute coefficient bound prevents clipping without automatic loudness gain.
struct ExtendedOscillator {
    sine: [f64; 64],
    cosine: [f64; 64],
    count: usize,
}

impl ExtendedOscillator {
    fn new(shape: ToneShape, maximum_frequency: f32) -> Self {
        let mut result = Self {
            sine: [0.0; 64],
            cosine: [0.0; 64],
            count: 0,
        };
        let width = shape.pulse_width.unwrap_or(0.5) as f64;
        let bound: f64 = (1..=64)
            .map(|n| {
                if shape.waveform == Waveform::Pulse {
                    (PI * n as f64 * width).sin().abs() / n as f64
                } else {
                    1.0 / n as f64
                }
            })
            .sum();
        for n in 1..=64 {
            if n as f32 * maximum_frequency >= SAMPLE_RATE as f32 / 2.0 {
                break;
            }
            if shape.waveform == Waveform::Pulse {
                let angle = TAU * n as f64 * width;
                result.cosine[n - 1] = angle.sin() / (2.0 * n as f64 * bound);
                result.sine[n - 1] = (1.0 - angle.cos()) / (2.0 * n as f64 * bound);
            } else {
                result.sine[n - 1] = -1.0 / (n as f64 * bound);
            }
            result.count += 1;
        }
        result
    }

    fn sample(&self, phase: f64) -> f32 {
        let (step_sine, step_cosine) = phase.sin_cos();
        let (mut sine, mut cosine) = (step_sine, step_cosine);
        let mut value = 0.0;
        for n in 0..self.count {
            value += self.sine[n] * sine + self.cosine[n] * cosine;
            (sine, cosine) = (
                sine * step_cosine + cosine * step_sine,
                cosine * step_cosine - sine * step_sine,
            );
        }
        value as f32
    }
}

struct Envelope {
    curve: EnvelopeCurve,
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
            curve: EnvelopeCurve::Linear,
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

    fn rise(&self, fraction: f32) -> f32 {
        match self.curve {
            EnvelopeCurve::Linear => fraction,
            EnvelopeCurve::Exponential => (4.0 * fraction).exp_m1() / 4.0_f32.exp_m1(),
        }
    }

    fn level(&self, frame: usize) -> f32 {
        let level = if frame < self.attack_end {
            self.rise(frame as f32 / self.attack_end as f32)
        } else if frame < self.decay_end {
            let fraction =
                (frame - self.attack_end) as f32 / (self.decay_end - self.attack_end) as f32;
            match self.curve {
                EnvelopeCurve::Linear => 1.0 + (self.sustain - 1.0) * fraction,
                EnvelopeCurve::Exponential => {
                    self.sustain + (1.0 - self.sustain) * self.rise(1.0 - fraction)
                }
            }
        } else if frame < self.release_start || self.release_start == self.last {
            self.sustain
        } else {
            self.sustain
                * self.rise((self.last - frame) as f32 / (self.last - self.release_start) as f32)
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
