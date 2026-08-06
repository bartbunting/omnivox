//! Descriptions shared by TTS engines, voice configuration, and discovery.
//!
//! These types are additive to the current [`crate::TtsEngine`] interface. They
//! define the richer contract without changing the legacy synthesis path yet.

use crate::VoiceQuality;

/// Stable identity for a physical voice.
///
/// Engine and voice IDs remain separate because native voice IDs may contain
/// colons, backslashes, or other separator characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhysicalVoiceId {
    pub engine_id: String,
    pub voice_id: String,
}

impl PhysicalVoiceId {
    pub fn new(engine_id: impl Into<String>, voice_id: impl Into<String>) -> Self {
        Self {
            engine_id: engine_id.into(),
            voice_id: voice_id.into(),
        }
    }
}

/// Whether an engine or voice can currently be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable { reason: String },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Runtime health of an available engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineHealth {
    Healthy,
    Degraded { reason: String },
    Failed { reason: String },
}

impl EngineHealth {
    /// A degraded engine may still synthesize and therefore remains eligible.
    pub fn can_synthesize(&self) -> bool {
        !matches!(self, Self::Failed { .. })
    }
}

/// How an engine supplies audio to Omnivox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOutputMode {
    BufferedPcm,
    StreamingPcm,
    ExternalPlayback,
}

/// The strongest cancellation guarantee an engine offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationSupport {
    None,
    PlaybackOnly,
    SynthesisAndPlayback,
}

/// Whether synthesis calls must be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyModel {
    Serialized,
    Concurrent { maximum_requests: Option<usize> },
}

/// Marker metadata an engine can return with synthesized audio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarkerCapabilities {
    pub word: bool,
    pub sentence: bool,
    pub phoneme: bool,
    pub native_index: bool,
}

/// Normalized ACSS dimensions understood by Omnivox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcssDimension {
    Rate,
    AveragePitch,
    PitchRange,
    Stress,
    Richness,
    Volume,
}

/// ACSS dimensions that an engine can apply or approximate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AcssCapabilities {
    pub rate: bool,
    pub average_pitch: bool,
    pub pitch_range: bool,
    pub stress: bool,
    pub richness: bool,
    pub volume: bool,
}

impl AcssCapabilities {
    pub fn supports(&self, dimension: AcssDimension) -> bool {
        match dimension {
            AcssDimension::Rate => self.rate,
            AcssDimension::AveragePitch => self.average_pitch,
            AcssDimension::PitchRange => self.pitch_range,
            AcssDimension::Stress => self.stress,
            AcssDimension::Richness => self.richness,
            AcssDimension::Volume => self.volume,
        }
    }
}

/// A normalized ACSS style. Present values are in the inclusive range 0.0..=1.0.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NormalizedAcss {
    pub rate: Option<f32>,
    pub average_pitch: Option<f32>,
    pub pitch_range: Option<f32>,
    pub stress: Option<f32>,
    pub richness: Option<f32>,
    pub volume: Option<f32>,
}

impl NormalizedAcss {
    /// Clamp all present values into the normalized range.
    pub fn clamped(mut self) -> Self {
        self.rate = self.rate.map(clamp_normalized);
        self.average_pitch = self.average_pitch.map(clamp_normalized);
        self.pitch_range = self.pitch_range.map(clamp_normalized);
        self.stress = self.stress.map(clamp_normalized);
        self.richness = self.richness.map(clamp_normalized);
        self.volume = self.volume.map(clamp_normalized);
        self
    }

    /// Omit unsupported dimensions while recording exactly what degraded.
    pub fn degrade_for(self, capabilities: &AcssCapabilities) -> AcssApplication {
        let mut style = self.clamped();
        let mut omitted = Vec::new();

        omit_unsupported(
            &mut style.rate,
            AcssDimension::Rate,
            capabilities,
            &mut omitted,
        );
        omit_unsupported(
            &mut style.average_pitch,
            AcssDimension::AveragePitch,
            capabilities,
            &mut omitted,
        );
        omit_unsupported(
            &mut style.pitch_range,
            AcssDimension::PitchRange,
            capabilities,
            &mut omitted,
        );
        omit_unsupported(
            &mut style.stress,
            AcssDimension::Stress,
            capabilities,
            &mut omitted,
        );
        omit_unsupported(
            &mut style.richness,
            AcssDimension::Richness,
            capabilities,
            &mut omitted,
        );
        omit_unsupported(
            &mut style.volume,
            AcssDimension::Volume,
            capabilities,
            &mut omitted,
        );

        AcssApplication { style, omitted }
    }
}

fn clamp_normalized(value: f32) -> f32 {
    if value.is_nan() {
        0.5
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn omit_unsupported(
    value: &mut Option<f32>,
    dimension: AcssDimension,
    capabilities: &AcssCapabilities,
    omitted: &mut Vec<AcssDimension>,
) {
    if value.is_some() && !capabilities.supports(dimension) {
        *value = None;
        omitted.push(dimension);
    }
}

/// ACSS values an engine can apply and the values omitted during degradation.
#[derive(Debug, Clone, PartialEq)]
pub struct AcssApplication {
    pub style: NormalizedAcss,
    pub omitted: Vec<AcssDimension>,
}

/// A namespaced optional native capability advertised by an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeExtensionDescriptor {
    pub id: String,
    pub description: String,
}

/// Capabilities used to route requests and degrade unsupported features.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineCapabilities {
    pub acss: AcssCapabilities,
    pub audio_output: AudioOutputMode,
    pub cancellation: CancellationSupport,
    pub concurrency: ConcurrencyModel,
    pub markers: MarkerCapabilities,
    pub language_switching: bool,
    pub native_extensions: Vec<NativeExtensionDescriptor>,
}

/// Coarse voice traits that may be used in portable selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceGender {
    Female,
    Male,
    Neutral,
}

/// A physical voice discovered from one engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceDescriptor {
    pub id: PhysicalVoiceId,
    pub display_name: String,
    pub language: Option<String>,
    pub gender: Option<VoiceGender>,
    pub quality: VoiceQuality,
    pub availability: Availability,
}

/// Complete runtime description of one speech engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDescriptor {
    pub id: String,
    pub display_name: String,
    pub version: Option<String>,
    pub availability: Availability,
    pub health: EngineHealth,
    pub capabilities: EngineCapabilities,
    pub voices: Vec<VoiceDescriptor>,
    pub default_voice_id: Option<String>,
}

impl EngineDescriptor {
    pub fn can_synthesize(&self) -> bool {
        self.availability.is_available() && self.health.can_synthesize()
    }
}

/// Portable or exact request for a physical voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceSelector {
    Exact(PhysicalVoiceId),
    EngineDefault {
        engine_id: String,
    },
    Properties {
        engine_id: Option<String>,
        language: Option<String>,
        gender: Option<VoiceGender>,
    },
}

impl VoiceSelector {
    pub fn engine_id(&self) -> Option<&str> {
        match self {
            Self::Exact(id) => Some(&id.engine_id),
            Self::EngineDefault { engine_id } => Some(engine_id),
            Self::Properties { engine_id, .. } => engine_id.as_deref(),
        }
    }
}

/// Stable Emacs/ACSS voice name and its portable ordered preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct LogicalVoiceDefinition {
    pub id: String,
    pub language: Option<String>,
    pub preferences: Vec<VoiceSelector>,
    pub acss: NormalizedAcss,
}

/// Machine/session policy applied after a logical voice's explicit preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackPolicy {
    pub allow_same_language_on_requested_engine: bool,
    pub global_default: Option<VoiceSelector>,
    pub fallback_engines: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_voice_identity_keeps_engine_and_native_id_separate() {
        let id = PhysicalVoiceId::new("winrt", r"winrt:HKEY_LOCAL_MACHINE\Voices\David");

        assert_eq!(id.engine_id, "winrt");
        assert_eq!(id.voice_id, r"winrt:HKEY_LOCAL_MACHINE\Voices\David");
    }

    #[test]
    fn normalized_acss_clamps_values_and_replaces_nan() {
        let style = NormalizedAcss {
            rate: Some(-0.2),
            average_pitch: Some(1.5),
            stress: Some(f32::NAN),
            ..NormalizedAcss::default()
        }
        .clamped();

        assert_eq!(style.rate, Some(0.0));
        assert_eq!(style.average_pitch, Some(1.0));
        assert_eq!(style.stress, Some(0.5));
    }

    #[test]
    fn normalized_acss_records_unsupported_dimensions() {
        let style = NormalizedAcss {
            rate: Some(0.7),
            richness: Some(0.8),
            volume: Some(0.6),
            ..NormalizedAcss::default()
        };
        let capabilities = AcssCapabilities {
            rate: true,
            volume: true,
            ..AcssCapabilities::default()
        };

        let application = style.degrade_for(&capabilities);

        assert_eq!(application.style.rate, Some(0.7));
        assert_eq!(application.style.richness, None);
        assert_eq!(application.style.volume, Some(0.6));
        assert_eq!(application.omitted, vec![AcssDimension::Richness]);
    }
}
