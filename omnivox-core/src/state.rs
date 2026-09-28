//! TTS State Management
//!
//! Manages application state including voice settings, volumes, and audio routing.

use std::sync::Arc;
use std::time::Duration;

use crate::settings::{CapitalPitchSettings, ChunkWordLimit, SpeechDefaults};
use serde::{Deserialize, Serialize};

/// Punctuation reading level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PunctuationLevel {
    /// Read only dollar sign and percent
    None,
    /// Read some punctuation marks
    Some,
    /// Read all punctuation marks
    All,
}

impl PunctuationLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "some" => Some(Self::Some),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Legacy isolated-capital presentation retained for command compatibility.
///
/// Current isolated letters use a pitch rise. Structured capitalization
/// presentation is carried by concrete timeline actions instead of this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapitalizationPresentation {
    /// Speak the character without an additional capitalization cue.
    None,
    /// Say "cap" before the character.
    Spoken,
    /// Overlay the standard capital tone at the character boundary.
    Tone,
    /// Say "cap" and overlay the standard capital tone.
    SpokenTone,
    /// Reserve presentation for caller-supplied semantic actions.
    Custom,
}

impl CapitalizationPresentation {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "spoken" => Some(Self::Spoken),
            "tone" => Some(Self::Tone),
            "spoken-tone" => Some(Self::SpokenTone),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn includes_spoken(self) -> bool {
        matches!(self, Self::Spoken | Self::SpokenTone)
    }

    pub fn includes_tone(self) -> bool {
        matches!(self, Self::Tone | Self::SpokenTone)
    }
}

/// Audio channel mode for stereo panning
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// Output to left channel only
    Left,
    /// Output to right channel only
    Right,
    /// Output to both channels (stereo)
    Both,
}

impl ChannelMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// Audio routing configuration
///
/// Specifies which device and channels to use for audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioRouting {
    /// Audio device ID (0 = system default)
    pub device_id: u32,
    /// Channel mode (left/right/both)
    pub channel_mode: ChannelMode,
}

impl AudioRouting {
    pub fn new(device_id: u32, channel_mode: ChannelMode) -> Self {
        Self {
            device_id,
            channel_mode,
        }
    }
}

impl Default for AudioRouting {
    fn default() -> Self {
        Self {
            device_id: 0,
            channel_mode: ChannelMode::Both,
        }
    }
}

/// TTS engine state
///
/// Maintains all configuration and state for the TTS engine.
#[derive(Debug, Clone)]
pub struct TtsState {
    /// Immutable saved values restored by reset, shared by admitted requests.
    pub speech_defaults: Arc<SpeechDefaults>,
    /// Host-owned startup preference, retained by client resets.
    pub max_chunk_words: ChunkWordLimit,
    /// Host cue policy, selected again for each actual engine attempt.
    pub capital_pitch: Arc<CapitalPitchSettings>,
    // Voice settings
    pub current_voice: String,
    pub pitch_multiplier: f32,
    pub speech_rate: f32,

    // Punctuation
    pub punctuation_level: PunctuationLevel,
    pub split_caps: bool,
    pub capitalization_presentation: CapitalizationPresentation,

    // Volume controls (0.0 to 1.0)
    pub voice_volume: f32,
    pub tone_volume: f32,
    pub sound_volume: f32,

    // Character rate
    pub character_scale: f32,

    // Delays
    pub pre_delay: Duration,
    pub post_delay: Duration,
    pub next_pre_delay: Duration,

    // Audio routing
    pub speech_routing: AudioRouting,
    pub tone_routing: AudioRouting,
    pub sound_routing: AudioRouting,
}

impl Default for TtsState {
    fn default() -> Self {
        Self::from_speech_defaults(Arc::new(SpeechDefaults::default()))
    }
}

impl TtsState {
    pub fn from_speech_defaults(defaults: Arc<SpeechDefaults>) -> Self {
        Self {
            max_chunk_words: ChunkWordLimit::default(),
            capital_pitch: Arc::new(CapitalPitchSettings::default()),
            current_voice: defaults.voice.clone().unwrap_or_else(|| "en-US".into()),
            pitch_multiplier: defaults.pitch,
            speech_rate: defaults.rate,
            punctuation_level: defaults.punctuation,
            split_caps: defaults.split_caps,
            capitalization_presentation: CapitalizationPresentation::None,
            voice_volume: defaults.voice_volume,
            tone_volume: defaults.tone_volume,
            sound_volume: defaults.sound_volume,
            character_scale: defaults.character_scale,
            speech_defaults: defaults,
            pre_delay: Duration::ZERO,
            post_delay: Duration::ZERO,
            next_pre_delay: Duration::ZERO,
            speech_routing: AudioRouting::default(),
            tone_routing: AudioRouting::default(),
            sound_routing: AudioRouting::default(),
        }
    }
}

impl TtsState {
    /// Create a new TTS state with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore saved speech defaults without rereading configuration files.
    pub fn reset(&mut self) {
        *self = Self {
            max_chunk_words: self.max_chunk_words,
            capital_pitch: Arc::clone(&self.capital_pitch),
            ..Self::from_speech_defaults(Arc::clone(&self.speech_defaults))
        };
    }

    /// Get the character speaking rate (speech_rate * character_scale)
    pub fn character_rate(&self) -> f32 {
        self.speech_rate * self.character_scale
    }

    /// Route every real output stream owned by this server process.
    ///
    /// A separately routed notification stream is a second process with its
    /// own state, not a write-only routing slot in this one.
    pub fn set_process_channel_mode(&mut self, channel_mode: ChannelMode) {
        self.speech_routing.channel_mode = channel_mode;
        self.tone_routing.channel_mode = channel_mode;
        self.sound_routing.channel_mode = channel_mode;
    }

    /// Consume and return the next pre-delay, resetting it to zero
    pub fn consume_next_pre_delay(&mut self) -> Duration {
        let delay = self.next_pre_delay;
        self.next_pre_delay = Duration::ZERO;
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = TtsState::default();
        assert_eq!(state.current_voice, "en-US");
        assert_eq!(state.pitch_multiplier, 1.0);
        assert_eq!(state.speech_rate, 0.5);
        assert_eq!(state.punctuation_level, PunctuationLevel::All);
        assert!(state.split_caps);
        assert_eq!(
            state.capitalization_presentation,
            CapitalizationPresentation::None
        );
    }

    #[test]
    fn test_character_rate() {
        let state = TtsState::default();
        assert_eq!(state.character_rate(), 0.5 * 1.2);
    }

    #[test]
    fn process_channel_mode_routes_every_owned_output_stream() {
        let mut state = TtsState::default();

        state.set_process_channel_mode(ChannelMode::Right);

        assert_eq!(state.speech_routing.channel_mode, ChannelMode::Right);
        assert_eq!(state.tone_routing.channel_mode, ChannelMode::Right);
        assert_eq!(state.sound_routing.channel_mode, ChannelMode::Right);
    }

    #[test]
    fn test_consume_next_pre_delay() {
        let mut state = TtsState {
            next_pre_delay: Duration::from_millis(100),
            ..TtsState::default()
        };

        let delay = state.consume_next_pre_delay();
        assert_eq!(delay, Duration::from_millis(100));
        assert_eq!(state.next_pre_delay, Duration::ZERO);
    }

    #[test]
    fn test_reset() {
        let mut state = TtsState {
            max_chunk_words: ChunkWordLimit::try_from(30).unwrap(),
            current_voice: String::from("en-GB:Daniel"),
            pitch_multiplier: 1.5,
            ..TtsState::default()
        };

        state.reset();
        assert_eq!(state.current_voice, "en-US");
        assert_eq!(state.pitch_multiplier, 1.0);
        assert_eq!(state.max_chunk_words.get(), 30);
    }

    #[test]
    fn reset_restores_saved_speech_and_clears_transient_state() {
        let defaults = Arc::new(SpeechDefaults {
            voice: Some("saved voice".into()),
            rate: 0.75,
            pitch: 1.3,
            voice_volume: 0.8,
            tone_volume: 0.2,
            sound_volume: 0.4,
            punctuation: PunctuationLevel::Some,
            split_caps: false,
            character_scale: 1.5,
        });
        let mut state = TtsState::from_speech_defaults(Arc::clone(&defaults));
        state.max_chunk_words = ChunkWordLimit::try_from(30).unwrap();
        state.capital_pitch = Arc::new(CapitalPitchSettings {
            default: crate::settings::CapitalPitch::Off,
            engines: std::collections::BTreeMap::from([(
                "custom".into(),
                crate::settings::CapitalPitch::Value(1.8),
            )]),
        });
        // Admitted requests retain their own state when later commands change it.
        let admitted = state.clone();
        for _ in 0..2 {
            state.current_voice = "client voice".into();
            state.speech_rate = 0.2;
            state.pitch_multiplier = 0.9;
            state.voice_volume = 0.3;
            state.tone_volume = 0.7;
            state.sound_volume = 0.9;
            state.punctuation_level = PunctuationLevel::None;
            state.split_caps = true;
            state.character_scale = 0.5;
            state.pre_delay = Duration::from_secs(1);
            state.post_delay = Duration::from_secs(2);
            state.next_pre_delay = Duration::from_secs(3);
            state.capitalization_presentation = CapitalizationPresentation::Spoken;
            state.reset();
            assert_eq!(state.current_voice, "saved voice");
            assert_eq!(state.speech_rate, 0.75);
            assert_eq!(state.pitch_multiplier, 1.3);
            assert_eq!(
                [state.voice_volume, state.tone_volume, state.sound_volume],
                [0.8, 0.2, 0.4]
            );
            assert_eq!(state.punctuation_level, PunctuationLevel::Some);
            assert!(!state.split_caps);
            assert_eq!(state.character_rate(), 0.75 * 1.5);
            assert_eq!(
                [state.pre_delay, state.post_delay, state.next_pre_delay],
                [Duration::ZERO; 3]
            );
            assert_eq!(
                state.capitalization_presentation,
                CapitalizationPresentation::None
            );
            assert_eq!(state.max_chunk_words.get(), 30);
            assert!(Arc::ptr_eq(&state.speech_defaults, &defaults));
            assert!(Arc::ptr_eq(&state.capital_pitch, &admitted.capital_pitch));
            assert_eq!(
                state
                    .capital_pitch
                    .pitch_for("custom", state.pitch_multiplier),
                1.8
            );
            assert_eq!(
                state
                    .capital_pitch
                    .pitch_for("other", state.pitch_multiplier),
                1.3
            );
        }
        assert_eq!(admitted.current_voice, "saved voice");
        assert_eq!(admitted.speech_rate, 0.75);
    }

    #[test]
    fn test_punctuation_level_parse() {
        assert_eq!(
            PunctuationLevel::parse("none"),
            Some(PunctuationLevel::None)
        );
        assert_eq!(
            PunctuationLevel::parse("some"),
            Some(PunctuationLevel::Some)
        );
        assert_eq!(PunctuationLevel::parse("all"), Some(PunctuationLevel::All));
        assert_eq!(PunctuationLevel::parse("invalid"), None);
    }

    #[test]
    fn test_capitalization_presentation_parse_and_components() {
        assert_eq!(
            CapitalizationPresentation::parse("spoken-tone"),
            Some(CapitalizationPresentation::SpokenTone)
        );
        assert!(CapitalizationPresentation::Spoken.includes_spoken());
        assert!(!CapitalizationPresentation::Spoken.includes_tone());
        assert!(CapitalizationPresentation::Tone.includes_tone());
        assert!(!CapitalizationPresentation::Tone.includes_spoken());
        assert!(CapitalizationPresentation::parse("beep").is_none());
    }

    #[test]
    fn test_channel_mode_parse() {
        assert_eq!(ChannelMode::parse("left"), Some(ChannelMode::Left));
        assert_eq!(ChannelMode::parse("right"), Some(ChannelMode::Right));
        assert_eq!(ChannelMode::parse("both"), Some(ChannelMode::Both));
        assert_eq!(ChannelMode::parse("invalid"), None);
    }

    #[test]
    fn test_audio_routing() {
        let routing = AudioRouting::new(125, ChannelMode::Left);
        assert_eq!(routing.device_id, 125);
        assert_eq!(routing.channel_mode, ChannelMode::Left);
    }

    #[test]
    fn test_audio_routing_default() {
        let routing = AudioRouting::default();
        assert_eq!(routing.device_id, 0);
        assert_eq!(routing.channel_mode, ChannelMode::Both);
    }
}
