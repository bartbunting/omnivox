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
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdsrEnvelope {
    pub attack_ms: u32,
    pub decay_ms: u32,
    pub sustain_level: f32,
    pub release_ms: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToneShape {
    pub waveform: Waveform,
    /// Omission retains Classic fades. Explicit null is not an envelope.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_envelope"
    )]
    pub envelope: Option<AdsrEnvelope>,
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
