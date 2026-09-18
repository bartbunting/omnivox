//! TTS Engine Abstraction
//!
//! Platform-agnostic TTS trait and implementations for different platforms.

use thiserror::Error;

pub mod contracts;
pub mod control;
pub mod engine_parameters;
pub mod engine_registry;
pub mod engine_voice_choices;
#[cfg(feature = "espeak")]
pub mod espeak;
pub mod helper_engine;
pub mod helper_protocol;
pub mod logical_voices;
pub mod macos;
pub mod marker_protocol;
pub mod native_parameters;
pub mod native_synthesis;
#[cfg(feature = "piper")]
pub mod piper;
pub mod presentation;
pub mod rate_calibration;
pub mod resolver;
pub mod routing_policy;
pub mod synthesis;
pub mod timeline_protocol;
pub mod timeline_v4;
pub mod timeline_v5;
pub mod voice_choices;
pub mod voice_library;
pub mod voice_preview_v2;
pub mod voice_preview_v3;
pub mod windows;

pub use omnivox_audio::AudioBuffer;
pub use synthesis::{
    AnchorAffinity, AnchorResolution, RequestedAnchor, ResolvedAnchor, SynthesisCancellationToken,
    SynthesisMarker, SynthesisMarkerKind, SynthesisRequest, SynthesisResult,
    SynthesisStreamCompletion, SynthesisStreamSink, SynthesisStreamStart, MAX_SYNTHESIS_ANCHORS,
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
    /// Host speech rate (0.0 to 2.0, 0.5 = calibrated normal); engines may saturate lower.
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

    /// Nonblocking cache qualification for this engine object's current runtime.
    /// A positive token is stable only while metadata remains valid; runtime
    /// replacement must change it. None means unavailable/busy/unsupported.
    /// This memory-only check must never connect, query, load or wait for speech.
    /// Tokens are parent-local, not persisted or substituted for wire identity.
    fn parameter_cache_epoch(&self) -> Option<u64> {
        None
    }

    /// Read a bounded parameter catalogue page from the current worker only.
    /// Never synthesize, load a model or reconnect to satisfy this query.
    fn engine_parameters(
        &self,
        query: engine_parameters::CatalogueQuery,
    ) -> Result<engine_parameters::CatalogueResult, engine_parameters::CatalogueError> {
        engine_parameters::validate_query(&query)?;
        Ok(engine_parameters::unavailable(
            engine_parameters::CatalogueUnavailable::NotDescribed,
            "This engine does not describe native voice parameters",
        ))
    }

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

    /// Produce one structured request as ordered canonical PCM windows.
    ///
    /// Buffered engines inherit this compatibility implementation. Engines
    /// advertising `streaming_pcm` override it and emit while native synthesis
    /// remains active.
    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        let result = self.synthesize(request)?;
        synthesis::stream_buffered_result(
            request,
            self.descriptor().capabilities.markers.requested_anchors,
            result,
            sink,
        )
    }

    /// Execute an explicit native block, returning tentative adapter evidence.
    /// Strict requests fail on unsupported engines before ordinary synthesis.
    fn synthesize_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &native_synthesis::VoiceParameters,
    ) -> Result<(SynthesisResult, native_synthesis::NativeApplication), TtsError> {
        let application = native_synthesis::common_only(parameters)?;
        Ok((self.synthesize(request)?, application))
    }

    /// Stream explicit native settings; evidence must precede start, PCM and markers.
    /// The callback is tentative: callers retain normal PCM commitment rules.
    /// Unsupported adapters keep their ordinary progressive path for common-only
    /// requests instead of introducing whole-utterance buffering.
    fn synthesize_stream_with_parameters(
        &self,
        request: &SynthesisRequest,
        parameters: &native_synthesis::VoiceParameters,
        sink: &mut dyn SynthesisStreamSink,
        application: &mut dyn FnMut(&native_synthesis::NativeApplication),
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        let receipt = native_synthesis::common_only(parameters)?;
        application(&receipt);
        self.synthesize_stream(request, sink)
    }

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
