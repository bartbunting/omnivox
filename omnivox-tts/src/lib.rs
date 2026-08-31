//! TTS Engine Abstraction
//!
//! Platform-agnostic TTS trait and implementations for different platforms.

use thiserror::Error;

pub mod contracts;
pub mod control;
pub mod engine_registry;
#[cfg(feature = "espeak")]
pub mod espeak;
pub mod helper_engine;
pub mod helper_protocol;
pub mod logical_voices;
pub mod macos;
pub mod marker_protocol;
#[cfg(feature = "piper")]
pub mod piper;
pub mod presentation;
pub mod resolver;
pub mod routing_policy;
pub mod synthesis;
pub mod timeline_protocol;
pub mod windows;

pub use omnivox_audio::AudioBuffer;
pub use synthesis::{
    AnchorAffinity, AnchorResolution, RequestedAnchor, ResolvedAnchor, SynthesisCancellationToken,
    SynthesisMarker, SynthesisMarkerKind, SynthesisRequest, SynthesisResult, MAX_SYNTHESIS_ANCHORS,
    MAX_SYNTHESIS_ANCHOR_ID_BYTES,
};

/// TTS engine errors
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("Voice not found: {0}")]
    VoiceNotFound(String),

    #[error("Synthesis failed: {0}")]
    SynthesisFailed(String),

    #[error("Engine not available")]
    NotAvailable,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

/// Standard output sample rate
pub const STANDARD_SAMPLE_RATE: u32 = omnivox_audio::buffer::SAMPLE_RATE;
/// Standard output channel count (stereo)
pub const STANDARD_CHANNELS: u16 = omnivox_audio::buffer::CHANNELS;

/// Voice information
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceInfo {
    /// Unique voice identifier
    pub identifier: String,
    /// Display name
    pub name: String,
    /// Language code (e.g., "en-US")
    pub language: String,
    /// Voice quality level
    pub quality: VoiceQuality,
}

/// Voice quality levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceQuality {
    /// Compact/basic quality
    Compact,
    /// Enhanced quality
    Enhanced,
    /// Premium/highest quality
    Premium,
}

/// TTS synthesis settings
#[derive(Debug, Clone)]
pub struct TtsSettings {
    /// Voice identifier
    pub voice: String,
    /// Host speech rate (0.0 to 2.0, 0.5 = normal); engines may clamp lower.
    pub rate: f32,
    /// Pitch multiplier (0.5 to 2.0, 1.0 = normal)
    pub pitch: f32,
    /// Volume (0.0 to 1.0)
    pub volume: f32,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            voice: String::from("en-US"),
            rate: 0.5,
            pitch: 1.0,
            volume: 1.0,
        }
    }
}

/// Platform-agnostic TTS engine trait
pub trait TtsEngine: Send + Sync {
    /// Describe this engine, its current runtime state, and discovered voices.
    fn descriptor(&self) -> contracts::EngineDescriptor;

    /// Prepare an engine for a circuit-breaker recovery probe.
    ///
    /// In-process engines need no preparation. Engines backed by helper
    /// processes can override this to restart or reconnect the helper before
    /// the probe synthesis call.
    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        Ok(())
    }

    /// Synthesize one structured request and report realized output metadata.
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError>;

    /// Stop current synthesis
    fn stop(&self);

    /// Check if currently synthesizing
    fn is_speaking(&self) -> bool;

    /// List available voices
    fn available_voices(&self) -> Vec<VoiceInfo>;

    /// Get voice info by identifier
    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tts_settings_default() {
        let settings = TtsSettings::default();
        assert_eq!(settings.voice, "en-US");
        assert_eq!(settings.rate, 0.5);
        assert_eq!(settings.pitch, 1.0);
        assert_eq!(settings.volume, 1.0);
    }
}
