//! Descriptions shared by TTS engines, voice configuration, and discovery.
//!
//! Together with [`crate::SynthesisRequest`] and [`crate::SynthesisResult`],
//! these types define the engine discovery, selection, and synthesis contract.

use crate::{VoiceInfo, VoiceQuality};
use serde::{Deserialize, Serialize};

/// Stable identity for a physical voice.
///
/// Engine and voice IDs remain separate because native voice IDs may contain
/// colons, backslashes, or other separator characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioOutputMode {
    BufferedPcm,
    StreamingPcm,
    ExternalPlayback,
}

/// The strongest cancellation guarantee an engine offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationSupport {
    None,
    PlaybackOnly,
    SynthesisAndPlayback,
}

/// Whether synthesis calls must be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ConcurrencyModel {
    Serialized,
    Concurrent { maximum_requests: Option<usize> },
}

/// Marker metadata an engine can return with synthesized audio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerCapabilities {
    pub word: bool,
    pub sentence: bool,
    pub phoneme: bool,
    pub native_index: bool,
    /// Strongest source-text anchor placement this engine can provide.
    #[serde(default)]
    pub requested_anchors: AnchorSupport,
}

/// Strongest placement guarantee available for requested source-text anchors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSupport {
    #[default]
    None,
    WordBoundary,
    Exact,
}

/// Normalized ACSS dimensions understood by Omnivox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcssDimension {
    Rate,
    AveragePitch,
    PitchRange,
    Stress,
    Richness,
    Volume,
}

/// Engine-independent dimensions Omnivox can apply to returned PCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostSynthesisDimension {
    Gain,
    LowPass,
    HighPass,
    Pan,
    Chorus,
    Reverb,
    Echo,
}

/// Complete normalized state for Omnivox-owned post-synthesis processing.
///
/// Present values are clamped to 0.0..=1.0. Neutral values are gain 0.5,
/// low-pass 1.0, high-pass 0.0, pan 0.5, and zero chorus/reverb/echo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PostSynthesisStyle {
    pub gain: Option<f32>,
    pub low_pass: Option<f32>,
    pub high_pass: Option<f32>,
    pub pan: Option<f32>,
    pub chorus: Option<f32>,
    pub reverb: Option<f32>,
    pub echo: Option<f32>,
}

impl PostSynthesisStyle {
    pub fn clamped(mut self) -> Self {
        self.gain = self.gain.map(clamp_normalized);
        self.low_pass = self.low_pass.map(clamp_normalized);
        self.high_pass = self.high_pass.map(clamp_normalized);
        self.pan = self.pan.map(clamp_normalized);
        self.chorus = self.chorus.map(clamp_normalized);
        self.reverb = self.reverb.map(clamp_normalized);
        self.echo = self.echo.map(clamp_normalized);
        self
    }

    pub fn active_dimensions(&self) -> Vec<PostSynthesisDimension> {
        let mut active = Vec::new();
        if self.gain.is_some() {
            active.push(PostSynthesisDimension::Gain);
        }
        if self.low_pass.is_some() {
            active.push(PostSynthesisDimension::LowPass);
        }
        if self.high_pass.is_some() {
            active.push(PostSynthesisDimension::HighPass);
        }
        if self.pan.is_some() {
            active.push(PostSynthesisDimension::Pan);
        }
        if self.chorus.is_some() {
            active.push(PostSynthesisDimension::Chorus);
        }
        if self.reverb.is_some() {
            active.push(PostSynthesisDimension::Reverb);
        }
        if self.echo.is_some() {
            active.push(PostSynthesisDimension::Echo);
        }
        active
    }

    /// Retain only dimensions available on the selected engine/audio path.
    pub fn degrade_for(self, supported: &[PostSynthesisDimension]) -> PostSynthesisApplication {
        let mut style = self.clamped();
        let mut omitted = Vec::new();
        omit_post_synthesis(
            &mut style.gain,
            PostSynthesisDimension::Gain,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.low_pass,
            PostSynthesisDimension::LowPass,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.high_pass,
            PostSynthesisDimension::HighPass,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.pan,
            PostSynthesisDimension::Pan,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.chorus,
            PostSynthesisDimension::Chorus,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.reverb,
            PostSynthesisDimension::Reverb,
            supported,
            &mut omitted,
        );
        omit_post_synthesis(
            &mut style.echo,
            PostSynthesisDimension::Echo,
            supported,
            &mut omitted,
        );
        PostSynthesisApplication { style, omitted }
    }
}

fn omit_post_synthesis(
    value: &mut Option<f32>,
    dimension: PostSynthesisDimension,
    supported: &[PostSynthesisDimension],
    omitted: &mut Vec<PostSynthesisDimension>,
) {
    if value.is_some() && !supported.contains(&dimension) {
        *value = None;
        omitted.push(dimension);
    }
}

/// Post-synthesis state Omnivox can apply and dimensions it omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostSynthesisApplication {
    pub style: PostSynthesisStyle,
    pub omitted: Vec<PostSynthesisDimension>,
}

/// Dimensions implemented by the common buffered-PCM renderer.
pub fn buffered_post_synthesis_dimensions() -> Vec<PostSynthesisDimension> {
    vec![
        PostSynthesisDimension::Gain,
        PostSynthesisDimension::LowPass,
        PostSynthesisDimension::HighPass,
        PostSynthesisDimension::Pan,
        PostSynthesisDimension::Chorus,
        PostSynthesisDimension::Reverb,
        PostSynthesisDimension::Echo,
    ]
}

/// ACSS dimensions that an engine can apply or approximate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NormalizedAcss {
    pub rate: Option<f32>,
    pub average_pitch: Option<f32>,
    pub pitch_range: Option<f32>,
    pub stress: Option<f32>,
    pub richness: Option<f32>,
    pub volume: Option<f32>,
}

/// Smallest portable voice-rate adjustment, in points on the normalized
/// 0..=100 speech-rate scale.
pub const MIN_RATE_OFFSET_POINTS: i16 = -20;

/// Largest portable voice-rate adjustment, in points on the normalized
/// 0..=100 speech-rate scale.
pub const MAX_RATE_OFFSET_POINTS: i16 = 20;

/// Apply a signed point adjustment to a normalized speech rate.
///
/// For example, a base rate of `0.75` with an offset of `-1` produces
/// `0.74`; an offset of `4` produces `0.79`.
pub fn apply_rate_offset(base_rate: f32, offset_points: i16) -> f32 {
    (base_rate + f32::from(offset_points) / 100.0).clamp(0.0, 1.0)
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcssApplication {
    pub style: NormalizedAcss,
    pub omitted: Vec<AcssDimension>,
}

/// A namespaced optional native capability advertised by an engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeExtensionDescriptor {
    pub id: String,
    pub description: String,
}

/// Source-text repertoire an engine accepts without replacement or loss.
///
/// `Unknown` is the backward-compatible default for descriptors that predate
/// this capability.  Routing treats it as an ASCII-only guarantee rather than
/// assuming that arbitrary Unicode will survive an undocumented conversion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRepertoire {
    #[default]
    Unknown,
    Unicode,
    #[serde(rename = "windows_1252")]
    Windows1252,
    #[serde(rename = "iso_8859_1")]
    Iso8859_1,
    #[serde(rename = "koi8_r")]
    Koi8R,
}

impl TextRepertoire {
    /// Return the first source character this repertoire cannot encode.
    ///
    /// The offset is a UTF-8 byte offset, matching synthesis anchors and
    /// source-text marker ranges throughout the public engine contract.
    pub fn first_unsupported(self, text: &str) -> Option<(usize, char)> {
        text.char_indices()
            .find(|(_, character)| !self.supports_character(*character))
    }

    pub fn supports_text(self, text: &str) -> bool {
        self.first_unsupported(text).is_none()
    }

    fn supports_character(self, character: char) -> bool {
        match self {
            Self::Unicode => true,
            Self::Iso8859_1 => u32::from(character) <= 0xff,
            Self::Windows1252 => windows_1252_supports(character),
            Self::Koi8R => {
                character.is_ascii()
                    || matches!(character, '\u{0401}' | '\u{0451}')
                    || ('\u{0410}'..='\u{044f}').contains(&character)
            }
            Self::Unknown => character.is_ascii(),
        }
    }
}

fn windows_1252_supports(character: char) -> bool {
    character.is_ascii()
        || ('\u{00a0}'..='\u{00ff}').contains(&character)
        || matches!(
            character,
            '\u{20ac}'
                | '\u{201a}'
                | '\u{0192}'
                | '\u{201e}'
                | '\u{2026}'
                | '\u{2020}'
                | '\u{2021}'
                | '\u{02c6}'
                | '\u{2030}'
                | '\u{0160}'
                | '\u{2039}'
                | '\u{0152}'
                | '\u{017d}'
                | '\u{2018}'
                | '\u{2019}'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{2022}'
                | '\u{2013}'
                | '\u{2014}'
                | '\u{02dc}'
                | '\u{2122}'
                | '\u{0161}'
                | '\u{203a}'
                | '\u{0153}'
                | '\u{017e}'
                | '\u{0178}'
        )
}

/// Capabilities used to route requests and degrade unsupported features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub acss: AcssCapabilities,
    pub audio_output: AudioOutputMode,
    pub cancellation: CancellationSupport,
    pub concurrency: ConcurrencyModel,
    pub markers: MarkerCapabilities,
    pub language_switching: bool,
    /// Exact repertoire accepted by the engine's text input boundary.
    #[serde(default)]
    pub text_repertoire: TextRepertoire,
    /// Omnivox-owned dimensions available after this engine returns audio.
    #[serde(default)]
    pub post_synthesis_dimensions: Vec<PostSynthesisDimension>,
    pub native_extensions: Vec<NativeExtensionDescriptor>,
}

/// Coarse voice traits that may be used in portable selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceGender {
    Female,
    Male,
    Neutral,
}

/// A physical voice discovered from one engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceDescriptor {
    pub id: PhysicalVoiceId,
    pub display_name: String,
    pub language: Option<String>,
    pub gender: Option<VoiceGender>,
    pub quality: VoiceQuality,
    pub availability: Availability,
}

impl VoiceDescriptor {
    /// Promote legacy voice discovery data into the structured engine model.
    pub fn from_voice_info(engine_id: &str, voice: VoiceInfo) -> Self {
        let language = if voice.language.is_empty() {
            None
        } else {
            Some(voice.language)
        };
        Self {
            id: PhysicalVoiceId::new(engine_id, voice.identifier),
            display_name: voice.name,
            language,
            gender: None,
            quality: voice.quality,
            availability: Availability::Available,
        }
    }
}

/// Complete runtime description of one speech engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Describe a configured engine whose runtime could not be initialized.
    /// Voices and optional capabilities remain unadvertised until discovery succeeds.
    pub fn unavailable(id: impl Into<String>, reason: impl Into<String>) -> Self {
        let id = id.into();
        let reason = reason.into();
        Self {
            display_name: id.clone(),
            id,
            version: None,
            availability: Availability::Unavailable {
                reason: reason.clone(),
            },
            health: EngineHealth::Failed { reason },
            capabilities: EngineCapabilities {
                acss: AcssCapabilities::default(),
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::None,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: false,
                text_repertoire: TextRepertoire::Unknown,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices: Vec::new(),
            default_voice_id: None,
        }
    }

    pub fn can_synthesize(&self) -> bool {
        self.availability.is_available() && self.health.can_synthesize()
    }
}

/// Portable or exact request for a physical voice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogicalVoiceDefinition {
    pub id: String,
    pub language: Option<String>,
    pub preferences: Vec<VoiceSelector>,
    pub acss: NormalizedAcss,
    #[serde(default)]
    pub effects: PostSynthesisStyle,
}

/// Machine/session policy applied after a logical voice's explicit preferences.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackPolicy {
    #[serde(default)]
    pub preferred_engines: Vec<String>,
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
    fn relative_rate_uses_direct_points_and_clamps_at_scale_edges() {
        assert!((apply_rate_offset(0.75, -1) - 0.74).abs() < f32::EPSILON);
        assert!((apply_rate_offset(0.75, 4) - 0.79).abs() < f32::EPSILON);
        assert_eq!(apply_rate_offset(0.03, -20), 0.0);
        assert_eq!(apply_rate_offset(0.95, 20), 1.0);
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

    #[test]
    fn post_synthesis_style_clamps_and_degrades_independently() {
        let application = PostSynthesisStyle {
            gain: Some(1.5),
            pan: Some(-1.0),
            chorus: Some(2.0),
            reverb: Some(f32::NAN),
            ..PostSynthesisStyle::default()
        }
        .degrade_for(&[PostSynthesisDimension::Gain, PostSynthesisDimension::Reverb]);

        assert_eq!(application.style.gain, Some(1.0));
        assert_eq!(application.style.pan, None);
        assert_eq!(application.style.chorus, None);
        assert_eq!(application.style.reverb, Some(0.5));
        assert_eq!(
            application.omitted,
            vec![PostSynthesisDimension::Pan, PostSynthesisDimension::Chorus]
        );
    }

    #[test]
    fn legacy_voice_info_promotes_without_joining_identifiers() {
        let descriptor = VoiceDescriptor::from_voice_info(
            "winrt",
            VoiceInfo {
                identifier: r"winrt:HKEY\Voice".to_owned(),
                name: "David".to_owned(),
                language: "en-US".to_owned(),
                quality: VoiceQuality::Enhanced,
            },
        );

        assert_eq!(descriptor.id.engine_id, "winrt");
        assert_eq!(descriptor.id.voice_id, r"winrt:HKEY\Voice");
        assert_eq!(descriptor.language.as_deref(), Some("en-US"));
    }

    #[test]
    fn legacy_marker_inventory_defaults_requested_anchors_to_none() {
        let capabilities: MarkerCapabilities = serde_json::from_str(
            r#"{"word":true,"sentence":false,"phoneme":false,"native_index":false}"#,
        )
        .unwrap();

        assert_eq!(capabilities.requested_anchors, AnchorSupport::None);
    }

    #[test]
    fn text_repertoires_distinguish_single_byte_inputs_from_unicode() {
        assert!(TextRepertoire::Unicode.supports_text("日本 👋 e\u{301}"));
        assert!(TextRepertoire::Windows1252.supports_text("Élan — € Œ"));
        assert!(!TextRepertoire::Windows1252.supports_text("日本"));
        assert!(!TextRepertoire::Windows1252.supports_text("e\u{301}"));
        assert!(TextRepertoire::Iso8859_1.supports_text("café ÿ"));
        assert!(!TextRepertoire::Iso8859_1.supports_text("€"));
        assert!(TextRepertoire::Koi8R.supports_text("Привет, мир! Ёж."));
        assert!(!TextRepertoire::Koi8R.supports_text("Привет — мир"));
        assert!(!TextRepertoire::Koi8R.supports_text("Вітаю"));
        assert!(TextRepertoire::Unknown.supports_text("ASCII only"));
        assert!(!TextRepertoire::Unknown.supports_text("café"));
        assert_eq!(
            serde_json::to_string(&TextRepertoire::Windows1252).unwrap(),
            "\"windows_1252\""
        );
        assert_eq!(
            serde_json::to_string(&TextRepertoire::Iso8859_1).unwrap(),
            "\"iso_8859_1\""
        );
        assert_eq!(
            serde_json::to_string(&TextRepertoire::Koi8R).unwrap(),
            "\"koi8_r\""
        );
    }

    #[test]
    fn unsupported_text_position_uses_utf8_byte_offsets() {
        assert_eq!(
            TextRepertoire::Windows1252.first_unsupported("Élan 日本"),
            Some((6, '日'))
        );
    }

    #[test]
    fn legacy_capability_descriptors_default_to_an_ascii_only_guarantee() {
        let capabilities: EngineCapabilities = serde_json::from_str(
            r#"{
                "acss": {
                    "rate": false,
                    "average_pitch": false,
                    "pitch_range": false,
                    "stress": false,
                    "richness": false,
                    "volume": false
                },
                "audio_output": "buffered_pcm",
                "cancellation": "none",
                "concurrency": {"mode": "serialized"},
                "markers": {
                    "word": false,
                    "sentence": false,
                    "phoneme": false,
                    "native_index": false
                },
                "language_switching": false,
                "native_extensions": []
            }"#,
        )
        .unwrap();

        assert_eq!(capabilities.text_repertoire, TextRepertoire::Unknown);
        assert!(capabilities.text_repertoire.supports_text("legacy ASCII"));
        assert!(!capabilities.text_repertoire.supports_text("café"));
    }
}
