//! Validated host preferences shared by configuration and speech preparation.

use serde::{Deserialize, Serialize};

use crate::state::PunctuationLevel;

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
