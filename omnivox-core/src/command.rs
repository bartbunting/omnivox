//! Emacspeak Protocol Command Parser
//!
//! Parses commands from the Emacspeak protocol. Supports three formats:
//! 1. ID only: `s`, `d`, `version`
//! 2. Block args: `q {text}`, `c {codes}`
//! 3. Space args: `t 440 50`, `tts_set_speech_rate 225`

use once_cell::sync::Lazy;
use regex::Regex;
use thiserror::Error;

use crate::queue::TonePlacement;

/// Version of the capability-gated presentation-tone command.
pub const PRESENTATION_TONE_PROTOCOL_VERSION: u8 = 1;

/// Upper frequency accepted by both structured and queued presentation tones.
pub const MAX_PRESENTATION_TONE_FREQUENCY_HZ: f32 = 24_000.0;

/// Longest queued presentation tone accepted on the wire.
pub const MAX_PRESENTATION_TONE_DURATION_MS: u32 = 60_000;

/// Legacy commands retained only so migration failures are explicit.
///
/// Omnivox routes language through registered logical voices and routes a
/// distinct notification stream through a second process. Keep parsing these
/// historical commands until clients have had a deprecation cycle, but never
/// imply that they mutate server state.
pub const DEPRECATED_PROTOCOL_COMMANDS: &[&str] = &[
    "set_lang",
    "set_next_lang",
    "set_previous_lang",
    "set_preferred_lang",
    "tts_set_notification_channel",
];

/// Command parse errors
#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error("Empty command")]
    Empty,

    #[error("Unknown command: {0}")]
    Unknown(String),

    #[error("Invalid format: {0}")]
    InvalidFormat(String),
}

/// Emacspeak protocol command IDs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandId {
    // Core commands
    Queue,     // q - queue speech
    Code,      // c - queue inline codes
    Dispatch,  // d - dispatch queue
    Stop,      // s - stop speaking
    Letter,    // l - speak letter immediately
    Tone,      // t - queue tone
    AudioIcon, // a - queue audio icon
    PlaySound, // p - play sound immediately
    Silence,   // sh - queue silence

    // State management
    TtsSay,               // tts_say - speak immediately
    TtsSetPunctuations,   // tts_set_punctuations
    TtsSetSpeechRate,     // tts_set_speech_rate
    TtsSetCharacterScale, // tts_set_character_scale
    TtsSplitCaps,         // tts_split_caps
    TtsSetCapitalizationPresentation, // tts_set_capitalization_presentation
    TtsSyncState,         // tts_sync_state
    TtsReset,             // tts_reset
    Version,              // version
    OmnivoxControl,       // omnivox_control - versioned Base64-JSON control request
    EmacsvoxTx,           // emacsvox_tx - replaceable Base64 presentation transaction
    EmacsvoxTimeline,     // emacsvox_timeline - structured Base64-JSON presentation
    EmacsvoxTimelinePart, // emacsvox_timeline_part - one bounded V3 transport fragment
    EmacsvoxTone,         // emacsvox_tone - versioned presentation-clock tone
    EmacsvoxTrackedDispatch, // emacsvox_tracked_dispatch - dispatch with terminal playback status
    EmacsvoxMarkerDispatch, // emacsvox_marker_dispatch - dispatch with playback marker events

    // SwiftMac extensions
    TtsSetVoice,               // tts_set_voice
    TtsSetPitchMultiplier,     // tts_set_pitch_multiplier
    TtsSetSoundVolume,         // tts_set_sound_volume
    TtsSetToneVolume,          // tts_set_tone_volume
    TtsSetVoiceVolume,         // tts_set_voice_volume
    TtsSetSpeechChannel,       // tts_set_speech_channel
    TtsSetNotificationChannel, // deprecated tts_set_notification_channel
    TtsExit,                   // tts_exit

    // Deprecated global language state; logical voices own language routing.
    SetLang,          // set_lang
    SetNextLang,      // set_next_lang
    SetPreviousLang,  // set_previous_lang
    SetPreferredLang, // set_preferred_lang
}

impl CommandId {
    /// Parse command ID from string
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "q" => Some(Self::Queue),
            "c" => Some(Self::Code),
            "d" => Some(Self::Dispatch),
            "s" => Some(Self::Stop),
            "l" => Some(Self::Letter),
            "t" => Some(Self::Tone),
            "a" => Some(Self::AudioIcon),
            "p" => Some(Self::PlaySound),
            "sh" => Some(Self::Silence),
            "tts_say" => Some(Self::TtsSay),
            "tts_set_punctuations" => Some(Self::TtsSetPunctuations),
            "tts_set_speech_rate" => Some(Self::TtsSetSpeechRate),
            "tts_set_character_scale" => Some(Self::TtsSetCharacterScale),
            "tts_split_caps" => Some(Self::TtsSplitCaps),
            "tts_set_capitalization_presentation" => {
                Some(Self::TtsSetCapitalizationPresentation)
            }
            "tts_sync_state" => Some(Self::TtsSyncState),
            "tts_reset" => Some(Self::TtsReset),
            "version" => Some(Self::Version),
            "omnivox_control" => Some(Self::OmnivoxControl),
            "emacsvox_tx" => Some(Self::EmacsvoxTx),
            "emacsvox_timeline" => Some(Self::EmacsvoxTimeline),
            "emacsvox_timeline_part" => Some(Self::EmacsvoxTimelinePart),
            "emacsvox_tone" => Some(Self::EmacsvoxTone),
            "emacsvox_tracked_dispatch" => Some(Self::EmacsvoxTrackedDispatch),
            "emacsvox_marker_dispatch" => Some(Self::EmacsvoxMarkerDispatch),
            "tts_set_voice" => Some(Self::TtsSetVoice),
            "tts_set_pitch_multiplier" => Some(Self::TtsSetPitchMultiplier),
            "tts_set_sound_volume" => Some(Self::TtsSetSoundVolume),
            "tts_set_tone_volume" => Some(Self::TtsSetToneVolume),
            "tts_set_voice_volume" => Some(Self::TtsSetVoiceVolume),
            "tts_set_speech_channel" => Some(Self::TtsSetSpeechChannel),
            "tts_set_notification_channel" => Some(Self::TtsSetNotificationChannel),
            "tts_exit" => Some(Self::TtsExit),
            "set_lang" => Some(Self::SetLang),
            "set_next_lang" => Some(Self::SetNextLang),
            "set_previous_lang" => Some(Self::SetPreviousLang),
            "set_preferred_lang" => Some(Self::SetPreferredLang),
            _ => None,
        }
    }
}

/// Parsed Emacspeak command
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub id: CommandId,
    pub args: Option<String>,
}

/// Validated bounded tone values carried by the command protocol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneCommand {
    pub frequency_hz: f32,
    pub duration_ms: u32,
}

/// Validated presentation-clock tone carried by `emacsvox_tone`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationToneCommand {
    pub placement: TonePlacement,
    pub frequency_hz: f32,
    pub duration_ms: u32,
}

impl Command {
    /// Create a new command
    pub fn new(id: CommandId, args: Option<String>) -> Self {
        Self { id, args }
    }
}

/// Regex pattern for parsing Emacspeak protocol
/// Matches three formats:
/// 1. `cmd {arg}` - block args (supports multiline with [\s\S]*)
/// 2. `cmd arg` - space args
/// 3. `cmd` - no args
static COMMAND_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)^(?:(?P<block_id>[a-z_]+)\s+\{(?P<block_arg>[\s\S]*)\}|(?P<space_id>[a-z_]+)\s+(?P<space_arg>.+)|(?P<id>[a-z_]+))$")
        .expect("Invalid regex pattern")
});

/// Parse an Emacspeak protocol command line
///
/// # Examples
///
/// ```
/// use omnivox_core::command::{parse_command, CommandId};
///
/// // Queue command with block args
/// let cmd = parse_command("q {Hello World}").unwrap();
/// assert_eq!(cmd.id, CommandId::Queue);
/// assert_eq!(cmd.args, Some("Hello World".to_string()));
///
/// // Tone command with space args
/// let cmd = parse_command("t 440 50").unwrap();
/// assert_eq!(cmd.id, CommandId::Tone);
/// assert_eq!(cmd.args, Some("440 50".to_string()));
///
/// // Dispatch with no args
/// let cmd = parse_command("d").unwrap();
/// assert_eq!(cmd.id, CommandId::Dispatch);
/// assert_eq!(cmd.args, None);
/// ```
pub fn parse_command(line: &str) -> Result<Command, ParseError> {
    let line = line.trim();

    if line.is_empty() {
        return Err(ParseError::Empty);
    }

    let caps = COMMAND_REGEX
        .captures(line)
        .ok_or_else(|| ParseError::InvalidFormat(line.to_string()))?;

    // Try block format first (id {args})
    if let Some(block_id) = caps.name("block_id") {
        let id_str = block_id.as_str();
        let id =
            CommandId::from_str(id_str).ok_or_else(|| ParseError::Unknown(id_str.to_string()))?;

        let args = caps.name("block_arg").map(|m| m.as_str().to_string());

        return Ok(Command::new(id, args));
    }

    // Try space format (id args)
    if let Some(space_id) = caps.name("space_id") {
        let id_str = space_id.as_str();
        let id =
            CommandId::from_str(id_str).ok_or_else(|| ParseError::Unknown(id_str.to_string()))?;

        let args = caps.name("space_arg").map(|m| m.as_str().to_string());

        return Ok(Command::new(id, args));
    }

    // Try ID only format
    if let Some(id_match) = caps.name("id") {
        let id_str = id_match.as_str();
        let id =
            CommandId::from_str(id_str).ok_or_else(|| ParseError::Unknown(id_str.to_string()))?;

        return Ok(Command::new(id, None));
    }

    Err(ParseError::InvalidFormat(line.to_string()))
}

fn parse_bounded_tone_values(frequency: &str, duration: &str) -> Result<ToneCommand, String> {
    let frequency_hz = frequency
        .parse::<f32>()
        .map_err(|_| "tone frequency must be numeric".to_owned())?;
    if !frequency_hz.is_finite()
        || frequency_hz <= 0.0
        || frequency_hz > MAX_PRESENTATION_TONE_FREQUENCY_HZ
    {
        return Err(format!(
            "tone frequency must be greater than zero and at most {MAX_PRESENTATION_TONE_FREQUENCY_HZ} Hz"
        ));
    }
    let duration_ms = duration
        .parse::<u32>()
        .map_err(|_| "tone duration must be an integer".to_owned())?;
    if duration_ms == 0 || duration_ms > MAX_PRESENTATION_TONE_DURATION_MS {
        return Err(format!(
            "tone duration must be from 1 through {MAX_PRESENTATION_TONE_DURATION_MS} ms"
        ));
    }
    Ok(ToneCommand {
        frequency_hz,
        duration_ms,
    })
}

/// Parse and bound `FREQUENCY_HZ DURATION_MS` for the independent `t` command.
pub fn parse_tone_arguments(arguments: &str) -> Result<ToneCommand, String> {
    let fields = arguments.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        return Err("tone requires FREQUENCY_HZ DURATION_MS".to_owned());
    }
    parse_bounded_tone_values(fields[0], fields[1])
}

/// Parse and bound `VERSION MODE FREQUENCY_HZ DURATION_MS` tone arguments.
pub fn parse_presentation_tone_arguments(
    arguments: &str,
) -> Result<PresentationToneCommand, String> {
    let fields = arguments.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4 {
        return Err("presentation tone requires VERSION MODE FREQUENCY_HZ DURATION_MS".to_owned());
    }
    let version = fields[0]
        .parse::<u8>()
        .map_err(|_| "presentation tone version must be an integer".to_owned())?;
    if version != PRESENTATION_TONE_PROTOCOL_VERSION {
        return Err(format!(
            "unsupported presentation tone version {version}; supported version is {PRESENTATION_TONE_PROTOCOL_VERSION}"
        ));
    }
    let placement = match fields[1] {
        "insert" => TonePlacement::Insert,
        "overlay" => TonePlacement::Overlay,
        _ => return Err("presentation tone mode must be insert or overlay".to_owned()),
    };
    let tone = parse_bounded_tone_values(fields[2], fields[3])?;
    Ok(PresentationToneCommand {
        placement,
        frequency_hz: tone.frequency_hz,
        duration_ms: tone.duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_queue_with_block_args() {
        let cmd = parse_command("q {Hello World}").unwrap();
        assert_eq!(cmd.id, CommandId::Queue);
        assert_eq!(cmd.args, Some("Hello World".to_string()));
    }

    #[test]
    fn test_parse_queue_with_empty_block() {
        let cmd = parse_command("q {}").unwrap();
        assert_eq!(cmd.id, CommandId::Queue);
        assert_eq!(cmd.args, Some(String::new()));
    }

    #[test]
    fn test_parse_code_with_block_args() {
        let cmd = parse_command("c [{voice en-US:Samantha}]").unwrap();
        assert_eq!(cmd.id, CommandId::Code);
        assert_eq!(cmd.args, Some("[{voice en-US:Samantha}]".to_string()));
    }

    #[test]
    fn test_parse_tone_with_space_args() {
        let cmd = parse_command("t 440 50").unwrap();
        assert_eq!(cmd.id, CommandId::Tone);
        assert_eq!(cmd.args, Some("440 50".to_string()));
    }

    #[test]
    fn tone_parser_accepts_fractional_frequency_and_rejects_unbounded_audio() {
        let tone = parse_tone_arguments("297.3018 150").unwrap();
        assert!((tone.frequency_hz - 297.3018).abs() < f32::EPSILON);
        assert_eq!(tone.duration_ms, 150);

        for arguments in [
            "NaN 50",
            "-440 50",
            "24001 50",
            "440 0",
            "440 60001",
            "440 50 ignored",
        ] {
            assert!(
                parse_tone_arguments(arguments).is_err(),
                "unexpectedly accepted {arguments:?}"
            );
        }
    }

    #[test]
    fn presentation_tone_parser_preserves_explicit_clock_mode() {
        let cmd = parse_command("emacsvox_tone 1 insert 297.3018 150").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxTone);
        let tone = parse_presentation_tone_arguments(cmd.args.as_deref().unwrap()).unwrap();
        assert_eq!(tone.placement, TonePlacement::Insert);
        assert!((tone.frequency_hz - 297.3018).abs() < f32::EPSILON);
        assert_eq!(tone.duration_ms, 150);

        let overlay = parse_presentation_tone_arguments("1 overlay 880 35").unwrap();
        assert_eq!(overlay.placement, TonePlacement::Overlay);
    }

    #[test]
    fn presentation_tone_parser_rejects_versions_modes_and_unbounded_audio() {
        for arguments in [
            "2 insert 440 50",
            "1 independent 440 50",
            "1 insert NaN 50",
            "1 insert 24001 50",
            "1 insert 440 0",
            "1 insert 440 60001",
            "1 insert 440",
        ] {
            assert!(
                parse_presentation_tone_arguments(arguments).is_err(),
                "unexpectedly accepted {arguments:?}"
            );
        }
    }

    #[test]
    fn test_parse_audio_icon_preserves_quoted_tcl_word() {
        let cmd = parse_command(r#"a "/tmp/cue space/done.ogg""#).unwrap();
        assert_eq!(cmd.id, CommandId::AudioIcon);
        assert_eq!(cmd.args, Some(r#""/tmp/cue space/done.ogg""#.to_string()));
    }

    #[test]
    fn test_parse_omnivox_control_payload() {
        let cmd = parse_command("omnivox_control {eyJ0eXBlIjoiY2FwYWJpbGl0aWVzIn0=}").unwrap();
        assert_eq!(cmd.id, CommandId::OmnivoxControl);
        assert_eq!(
            cmd.args,
            Some("eyJ0eXBlIjoiY2FwYWJpbGl0aWVzIn0=".to_string())
        );
    }

    #[test]
    fn test_parse_emacsvox_transaction_arguments() {
        let cmd = parse_command("emacsvox_tx 17 {cSB7aGVsbG99XG5kXG4=}").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxTx);
        assert_eq!(cmd.args, Some("17 {cSB7aGVsbG99XG5kXG4=}".to_string()));
    }

    #[test]
    fn test_parse_emacsvox_timeline_payload() {
        let cmd = parse_command("emacsvox_timeline {eyJwcm90b2NvbF92ZXJzaW9uIjoxfQ==}").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxTimeline);
        assert_eq!(
            cmd.args,
            Some("eyJwcm90b2NvbF92ZXJzaW9uIjoxfQ==".to_string())
        );
    }

    #[test]
    fn test_parse_emacsvox_timeline_part() {
        let cmd = parse_command("emacsvox_timeline_part 3 17 29 0 2 300000 eyJwYXJ0").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxTimelinePart);
        assert_eq!(cmd.args.as_deref(), Some("3 17 29 0 2 300000 eyJwYXJ0"));
    }

    #[test]
    fn test_parse_emacsvox_tracked_dispatch_identifier() {
        let cmd = parse_command("emacsvox_tracked_dispatch 73").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxTrackedDispatch);
        assert_eq!(cmd.args, Some("73".to_string()));
    }

    #[test]
    fn test_parse_emacsvox_marker_dispatch_identifier() {
        let cmd = parse_command("emacsvox_marker_dispatch 91").unwrap();
        assert_eq!(cmd.id, CommandId::EmacsvoxMarkerDispatch);
        assert_eq!(cmd.args, Some("91".to_string()));
    }

    #[test]
    fn test_parse_tts_set_speech_rate() {
        let cmd = parse_command("tts_set_speech_rate 225").unwrap();
        assert_eq!(cmd.id, CommandId::TtsSetSpeechRate);
        assert_eq!(cmd.args, Some("225".to_string()));
    }

    #[test]
    fn test_parse_dispatch_no_args() {
        let cmd = parse_command("d").unwrap();
        assert_eq!(cmd.id, CommandId::Dispatch);
        assert_eq!(cmd.args, None);
    }

    #[test]
    fn test_parse_stop_no_args() {
        let cmd = parse_command("s").unwrap();
        assert_eq!(cmd.id, CommandId::Stop);
        assert_eq!(cmd.args, None);
    }

    #[test]
    fn test_parse_version_no_args() {
        let cmd = parse_command("version").unwrap();
        assert_eq!(cmd.id, CommandId::Version);
        assert_eq!(cmd.args, None);
    }

    #[test]
    fn test_parse_empty_line() {
        let result = parse_command("");
        assert_eq!(result, Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_whitespace_only() {
        let result = parse_command("   ");
        assert_eq!(result, Err(ParseError::Empty));
    }

    #[test]
    fn test_parse_unknown_command() {
        let result = parse_command("foobar");
        assert!(matches!(result, Err(ParseError::Unknown(_))));
    }

    #[test]
    fn test_parse_multiline_block_args() {
        let cmd = parse_command("q {Line 1\nLine 2\nLine 3}").unwrap();
        assert_eq!(cmd.id, CommandId::Queue);
        assert_eq!(cmd.args, Some("Line 1\nLine 2\nLine 3".to_string()));
    }

    #[test]
    fn test_parse_tts_sync_state() {
        let cmd = parse_command("tts_sync_state all 1 0 225").unwrap();
        assert_eq!(cmd.id, CommandId::TtsSyncState);
        assert_eq!(cmd.args, Some("all 1 0 225".to_string()));
    }

    #[test]
    fn test_parse_capitalization_presentation() {
        let cmd = parse_command("tts_set_capitalization_presentation spoken-tone").unwrap();
        assert_eq!(cmd.id, CommandId::TtsSetCapitalizationPresentation);
        assert_eq!(cmd.args, Some("spoken-tone".to_string()));
    }

    #[test]
    fn test_parse_silence() {
        let cmd = parse_command("sh 100").unwrap();
        assert_eq!(cmd.id, CommandId::Silence);
        assert_eq!(cmd.args, Some("100".to_string()));
    }

    #[test]
    fn test_parse_letter() {
        let cmd = parse_command("l A").unwrap();
        assert_eq!(cmd.id, CommandId::Letter);
        assert_eq!(cmd.args, Some("A".to_string()));
    }

    // Regression tests for ;; text (Lisp comment style)
    #[test]
    fn test_parse_semicolons_in_block_arg() {
        // Parser must preserve full text including ;; and everything after
        let cmd = parse_command(r#"q {hello ;; this is a comment}"#).unwrap();
        assert_eq!(cmd.args, Some("hello ;; this is a comment".to_string()));
    }

    #[test]
    fn test_parse_semicolons_with_quote_after() {
        // The reported bug: text after ;; was dropped until the next quote char
        let cmd = parse_command(r#"q {foo ;; bar "baz" }"#).unwrap();
        // Full text must be preserved
        let args = cmd.args.unwrap();
        assert!(args.contains(";;"), "semicolons must be preserved: {args}");
        assert!(args.contains("bar"), "text after ;; must be preserved: {args}");
        assert!(args.contains("baz"), "quoted text must be preserved: {args}");
    }

    #[test]
    fn test_parse_dtk_speak_format() {
        // dtk-speak.el sends: (format "q {%s }\n" text) -- trailing space before }
        let cmd = parse_command(r#"q {(setq x 1) ;; set x to 1 }"#).unwrap();
        let args = cmd.args.unwrap();
        assert!(args.contains(";;"), "semicolons preserved in dtk format: {args}");
        assert!(args.contains("set x to 1"), "comment text preserved: {args}");
    }

    #[test]
    fn deprecated_commands_remain_parseable_during_migration() {
        for name in DEPRECATED_PROTOCOL_COMMANDS {
            let command = parse_command(name).unwrap();
            assert!(matches!(
                command.id,
                CommandId::SetLang
                    | CommandId::SetNextLang
                    | CommandId::SetPreviousLang
                    | CommandId::SetPreferredLang
                    | CommandId::TtsSetNotificationChannel
            ));
        }
    }
}
