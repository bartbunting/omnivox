//! Shared bounds and typed sound shapes for richer generated tones.
//!
//! Decoding these values does not advertise or enable richer-tone playback.

use crate::command::{MAX_PRESENTATION_TONE_DURATION_MS, MAX_PRESENTATION_TONE_FREQUENCY_HZ};
use serde::{Deserialize, Serialize};

pub const MAX_SHAPED_TONE_FREQUENCY_HZ: f32 = 20_000.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Square,
    Pulse,
    Sawtooth,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdsrEnvelope {
    pub attack_ms: u32,
    pub decay_ms: u32,
    pub sustain_level: f32,
    pub release_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeCurve {
    Linear,
    Exponential,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToneShape {
    pub waveform: Waveform,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_number"
    )]
    pub pulse_width: Option<f32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_number"
    )]
    pub end_frequency_hz: Option<f32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_curve"
    )]
    pub envelope_curve: Option<EnvelopeCurve>,
    /// Omission retains Classic fades. Explicit null is not an envelope.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_envelope"
    )]
    pub envelope: Option<AdsrEnvelope>,
}

impl ToneShape {
    /// Version 2 standalone and version 6 timelines retain their exact shapes.
    pub fn requires_extended_contract(&self) -> bool {
        matches!(self.waveform, Waveform::Pulse | Waveform::Sawtooth)
            || self.pulse_width.is_some()
            || self.end_frequency_hz.is_some()
            || self.envelope_curve.is_some()
    }
}

fn present_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    f32::deserialize(d).map(Some)
}

fn present_curve<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<EnvelopeCurve>, D::Error> {
    EnvelopeCurve::deserialize(d).map(Some)
}

fn present_envelope<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<AdsrEnvelope>, D::Error> {
    AdsrEnvelope::deserialize(deserializer).map(Some)
}

/// One frozen sound definition, independent of clock placement and routing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneParameters {
    pub frequency_hz: f32,
    pub duration_ms: u32,
    pub volume: f32,
    pub shape: ToneShape,
}

impl ToneParameters {
    /// Shared standalone/timeline validation, including programmatic values.
    pub fn validate(&self) -> Result<(), String> {
        let maximum = if self.shape == ToneShape::default() {
            MAX_PRESENTATION_TONE_FREQUENCY_HZ
        } else {
            MAX_SHAPED_TONE_FREQUENCY_HZ
        };
        if !self.frequency_hz.is_finite() || self.frequency_hz <= 0.0 || self.frequency_hz > maximum
        {
            return Err(format!(
                "tone frequency must be finite, positive and at most {maximum} Hz"
            ));
        }
        if self.duration_ms == 0 || self.duration_ms > MAX_PRESENTATION_TONE_DURATION_MS {
            return Err("tone duration must be from 1 through 60000 ms".into());
        }
        if !self.volume.is_finite() || !(0.0..=1.0).contains(&self.volume) {
            return Err("tone volume must be finite and from 0 through 1".into());
        }
        match (self.shape.waveform, self.shape.pulse_width) {
            (Waveform::Pulse, Some(width))
                if width.is_finite() && (0.05..=0.95).contains(&width) => {}
            (Waveform::Pulse, _) => {
                return Err("pulse requires width from 0.05 through 0.95".into())
            }
            (_, Some(_)) => return Err("pulse width applies only to pulse".into()),
            _ => (),
        }
        if self
            .shape
            .end_frequency_hz
            .is_some_and(|end| !end.is_finite() || end <= 0.0 || end > MAX_SHAPED_TONE_FREQUENCY_HZ)
        {
            return Err("sweep end frequency must be positive and at most 20000 Hz".into());
        }
        if self.shape.envelope_curve.is_some() && self.shape.envelope.is_none() {
            return Err("envelope curve requires ADSR".into());
        }
        if let Some(envelope) = self.shape.envelope {
            if [envelope.attack_ms, envelope.decay_ms, envelope.release_ms]
                .into_iter()
                .any(|time| time > MAX_PRESENTATION_TONE_DURATION_MS)
            {
                return Err("tone envelope times must be from 0 through 60000 ms".into());
            }
            if !envelope.sustain_level.is_finite() || !(0.0..=1.0).contains(&envelope.sustain_level)
            {
                return Err("tone sustain level must be finite and from 0 through 1".into());
            }
        }
        Ok(())
    }
}
