//! Validated host preferences shared by configuration and speech preparation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::state::{ChannelMode, PunctuationLevel};

/// Existing host output methods; native support is checked when opening output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioBackend {
    /// Play through the system default output device in real time.
    Device,
    /// Use native PulseAudio on Linux.
    Pulse,
    /// Consume samples without opening a device or waiting for their duration.
    Null,
}

/// Requested PulseAudio latency. This is not a measured end-to-end latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct PulseLatencyMs(u16);

impl Default for PulseLatencyMs {
    fn default() -> Self {
        Self(20)
    }
}

impl TryFrom<u16> for PulseLatencyMs {
    type Error = &'static str;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (10..=200).contains(&value) {
            Ok(Self(value))
        } else {
            Err("PulseAudio latency must be an integer from 10 through 200")
        }
    }
}

impl From<PulseLatencyMs> for u16 {
    fn from(value: PulseLatencyMs) -> Self {
        value.0
    }
}

impl PulseLatencyMs {
    pub fn get(self) -> u32 {
        u32::from(self.0)
    }
}

/// Complete file baseline, captured with engine startup. Launcher overrides
/// remain per process and are applied when output is constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioOutputSettings {
    pub backend: AudioBackend,
    pub target: ChannelMode,
    pub pulse_latency_ms: PulseLatencyMs,
}

impl Default for AudioOutputSettings {
    fn default() -> Self {
        Self {
            backend: AudioBackend::Device,
            target: ChannelMode::Both,
            pulse_latency_ms: PulseLatencyMs::default(),
        }
    }
}

/// Absolute host pitch for an isolated capital, or no special pitch cue.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CapitalPitchWire", into = "CapitalPitchWire")]
pub enum CapitalPitch {
    Off,
    Value(f32),
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum CapitalPitchWire {
    Number(f32),
    Text(String),
}

impl TryFrom<CapitalPitchWire> for CapitalPitch {
    type Error = &'static str;
    fn try_from(value: CapitalPitchWire) -> Result<Self, Self::Error> {
        match value {
            CapitalPitchWire::Number(value)
                if value.is_finite() && (0.5..=2.0).contains(&value) =>
            {
                Ok(Self::Value(value))
            }
            CapitalPitchWire::Text(value) if value == "off" => Ok(Self::Off),
            _ => Err("capital pitch must be a number from 0.5 through 2.0 or off"),
        }
    }
}

impl From<CapitalPitch> for CapitalPitchWire {
    fn from(value: CapitalPitch) -> Self {
        match value {
            CapitalPitch::Off => Self::Text("off".into()),
            CapitalPitch::Value(value) => Self::Number(value),
        }
    }
}

/// Complete immutable cue policy. Public input may omit either member; private
/// startup records require both. Engine references are validated by the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapitalPitchSettings {
    pub default: CapitalPitch,
    pub engines: BTreeMap<String, CapitalPitch>,
}

impl Default for CapitalPitchSettings {
    fn default() -> Self {
        Self {
            default: CapitalPitch::Value(1.5),
            engines: BTreeMap::new(),
        }
    }
}

impl CapitalPitchSettings {
    pub fn pitch_for(&self, engine_id: &str, ordinary_pitch: f32) -> f32 {
        match self.engines.get(engine_id).unwrap_or(&self.default) {
            CapitalPitch::Off => ordinary_pitch,
            CapitalPitch::Value(value) => *value,
        }
    }
}

/// Complete saved speech defaults. Public configuration fills omitted members;
/// private startup records must carry every member, including nullable voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechDefaults {
    #[serde(deserialize_with = "required_voice")]
    pub voice: Option<String>,
    pub rate: f32,
    pub pitch: f32,
    pub voice_volume: f32,
    pub tone_volume: f32,
    pub sound_volume: f32,
    pub punctuation: PunctuationLevel,
    pub split_caps: bool,
    pub character_scale: f32,
}

fn required_voice<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

impl Default for SpeechDefaults {
    fn default() -> Self {
        Self {
            voice: None,
            rate: 0.5,
            pitch: 1.0,
            voice_volume: 1.0,
            tone_volume: 1.0,
            sound_volume: 1.0,
            punctuation: PunctuationLevel::All,
            split_caps: true,
            character_scale: 1.2,
        }
    }
}

impl SpeechDefaults {
    /// Return only the invalid field name; never expose configuration values.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.voice.as_ref().is_some_and(|voice| {
            voice.trim().is_empty() || voice.len() > 1024 || voice.chars().any(char::is_control)
        }) {
            return Err("voice");
        }
        for (field, value, min, max) in [
            ("rate", self.rate, 0.0, 2.0),
            ("pitch", self.pitch, 0.5, 2.0),
            ("voice_volume", self.voice_volume, 0.0, 1.0),
            ("tone_volume", self.tone_volume, 0.0, 1.0),
            ("sound_volume", self.sound_volume, 0.0, 1.0),
            ("character_scale", self.character_scale, 0.1, 4.0),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(field);
            }
        }
        Ok(())
    }
}

/// Maximum whitespace-delimited words in one prepared synthesis call.
/// Zero must never turn a configured limit into unlimited synthesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct ChunkWordLimit(u16);

impl ChunkWordLimit {
    pub fn get(self) -> usize {
        usize::from(self.0)
    }
}

impl Default for ChunkWordLimit {
    fn default() -> Self {
        Self(15)
    }
}

impl TryFrom<u16> for ChunkWordLimit {
    type Error = &'static str;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=100).contains(&value) {
            Ok(Self(value))
        } else {
            Err("max_chunk_words must be an integer from 1 through 100")
        }
    }
}

impl From<ChunkWordLimit> for u16 {
    fn from(value: ChunkWordLimit) -> Self {
        value.0
    }
}
