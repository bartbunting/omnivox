//! Text processing: punctuation expansion, CamelCase splitting, chunking, rate mapping.

use omnivox_core::{state::PunctuationLevel, TtsState};
use once_cell::sync::Lazy;
use std::path::PathBuf;

pub const CAPITAL_TONE_HZ: f32 = 440.0;
pub const ALL_CAPS_TONE_HZ: f32 = 1300.0;
pub const CAPITAL_TONE_DURATION_MS: u32 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct CapitalizationTone {
    pub id: String,
    pub text_offset: u32,
    pub frequency_hz: f32,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSpeechText {
    pub text: String,
    pub capitalization_tones: Vec<CapitalizationTone>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedSpeechChunk {
    pub text: String,
    pub capitalization_tones: Vec<CapitalizationTone>,
}

// ---------------------------------------------------------------------------
// Compiled regexes (one-time cost)
// ---------------------------------------------------------------------------

pub(crate) static VOICE_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\{voice\s+([^\}]+)\}\]").expect("invalid voice regex"));

pub(crate) static PITCH_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\[pitch\s+([^\]]+)\]\]").expect("invalid pitch regex"));

pub(crate) static LOGICAL_VOICE_RE: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"\[\[logical_voice\s+([A-Za-z0-9_.-]{1,128})\]\]")
        .expect("invalid logical voice regex")
});

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split prepared speech without losing capitalization anchor offsets.
pub fn chunk_prepared_speech(
    prepared: PreparedSpeechText,
    max_words: usize,
) -> Vec<PreparedSpeechChunk> {
    let spans = word_spans(&prepared.text);
    if spans.len() <= max_words || max_words == 0 {
        return vec![PreparedSpeechChunk {
            text: prepared.text,
            capitalization_tones: prepared.capitalization_tones,
        }];
    }

    spans
        .chunks(max_words)
        .map(|words| {
            let start = words.first().expect("word chunk is nonempty").0;
            let end = words.last().expect("word chunk is nonempty").1;
            let capitalization_tones = prepared
                .capitalization_tones
                .iter()
                .filter(|tone| {
                    let offset = tone.text_offset as usize;
                    offset >= start && offset < end
                })
                .cloned()
                .map(|mut tone| {
                    tone.text_offset -= start as u32;
                    tone
                })
                .collect();
            PreparedSpeechChunk {
                text: prepared.text[start..end].to_owned(),
                capitalization_tones,
            }
        })
        .collect()
}

fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (offset, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(start) = start.take() {
                spans.push((start, offset));
            }
        } else if start.is_none() {
            start = Some(offset);
        }
    }
    if let Some(start) = start {
        spans.push((start, text.len()));
    }
    spans
}

// ---------------------------------------------------------------------------
// Punctuation expansion
// ---------------------------------------------------------------------------

/// Replace punctuation characters with spoken names based on the active level.
pub fn apply_punctuation(text: &str, level: PunctuationLevel) -> String {
    let mut result = String::with_capacity(text.len());

    for ch in text.chars() {
        let replacement = match level {
            PunctuationLevel::None => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                _ => None,
            },
            PunctuationLevel::Some => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                '#' => Some(" pound "),
                '-' => Some(" dash "),
                '"' => Some(" quote "),
                '(' => Some(" left paren "),
                ')' => Some(" right paren "),
                '*' => Some(" star "),
                ';' => Some(" semicolon "),
                ':' => Some(" colon "),
                '<' => Some(" less than "),
                '>' => Some(" greater than "),
                '\\' => Some(" backslash "),
                '/' => Some(" slash "),
                '+' => Some(" plus "),
                '=' => Some(" equals "),
                '~' => Some(" tilde "),
                '`' => Some(" backquote "),
                '!' => Some(" bang "),
                '^' => Some(" caret "),
                _ => None,
            },
            PunctuationLevel::All => match ch {
                '$' => Some(" dollar "),
                '%' => Some(" percent "),
                '#' => Some(" pound "),
                '-' => Some(" dash "),
                '"' => Some(" quote "),
                '(' => Some(" left paren "),
                ')' => Some(" right paren "),
                '*' => Some(" star "),
                ';' => Some(" semicolon "),
                ':' => Some(" colon "),
                '<' => Some(" less than "),
                '>' => Some(" greater than "),
                '\\' => Some(" backslash "),
                '/' => Some(" slash "),
                '+' => Some(" plus "),
                '=' => Some(" equals "),
                '~' => Some(" tilde "),
                '`' => Some(" backquote "),
                '!' => Some(" bang "),
                '^' => Some(" caret "),
                '@' => Some(" at "),
                '_' => Some(" underline "),
                '\'' => Some(" apostrophe "),
                '.' => Some(" dot "),
                ',' => Some(" comma "),
                '&' => Some(" ampersand "),
                '|' => Some(" pipe "),
                '[' => Some(" left bracket "),
                ']' => Some(" right bracket "),
                '{' => Some(" left brace "),
                '}' => Some(" right brace "),
                '?' => Some(" question "),
                _ => None,
            },
        };

        match replacement {
            Some(spoken) => result.push_str(spoken),
            None => result.push(ch),
        }
    }

    result
}

// ---------------------------------------------------------------------------
// CamelCase splitting
// ---------------------------------------------------------------------------

/// Insert a space before each uppercase letter that follows a lowercase letter.
///
/// This splits CamelCase identifiers (e.g. `helloWorld` → `hello World`)
/// while leaving acronyms intact (e.g. `HTTPServer` → `HTTPServer`).
/// Matches the behaviour of SwiftMac's `(?<=[a-z])(?=[A-Z])` pattern.
pub fn insert_space_before_uppercase(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 2);
    let mut prev_lower = false;
    for c in input.chars() {
        if c.is_uppercase() && prev_lower {
            result.push(' ');
        }
        prev_lower = c.is_lowercase();
        result.push(c);
    }
    result
}

// ---------------------------------------------------------------------------
// Text preprocessing
// ---------------------------------------------------------------------------

/// Apply speech preprocessing while retaining capitalization tone positions.
pub fn prepare_speech_text(text: &str, state: &TtsState) -> PreparedSpeechText {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    if state.allcaps_beep {
        annotate_capitalization(&processed)
    } else {
        PreparedSpeechText {
            text: processed,
            capitalization_tones: Vec::new(),
        }
    }
}

fn annotate_capitalization(input: &str) -> PreparedSpeechText {
    let mut text = String::with_capacity(input.len());
    let mut tones = Vec::new();
    let mut position = 0;
    while position < input.len() {
        let character = input[position..]
            .chars()
            .next()
            .expect("position remains on a character boundary");
        if character.is_uppercase() && is_word_start(input, position) {
            if let Some(end) = all_caps_run_end(input, position) {
                let id = format!("capitalization-{}", tones.len());
                tones.push(CapitalizationTone {
                    id,
                    text_offset: text.len() as u32,
                    frequency_hz: ALL_CAPS_TONE_HZ,
                    duration_ms: CAPITAL_TONE_DURATION_MS,
                });
                text.extend(input[position..end].chars().flat_map(char::to_lowercase));
                position = end;
                continue;
            }
        }
        if character.is_uppercase() {
            let id = format!("capitalization-{}", tones.len());
            tones.push(CapitalizationTone {
                id,
                text_offset: text.len() as u32,
                frequency_hz: CAPITAL_TONE_HZ,
                duration_ms: CAPITAL_TONE_DURATION_MS,
            });
        }
        text.push(character);
        position += character.len_utf8();
    }
    PreparedSpeechText {
        text,
        capitalization_tones: tones,
    }
}

fn is_word_start(text: &str, position: usize) -> bool {
    text[..position]
        .chars()
        .next_back()
        .is_none_or(|character| !is_word_character(character))
}

fn all_caps_run_end(text: &str, start: usize) -> Option<usize> {
    let mut end = start;
    let mut count = 0;
    for character in text[start..].chars() {
        if character.is_uppercase()
            || character.is_numeric()
            || character == '_'
            || character == '-'
        {
            end += character.len_utf8();
            count += 1;
        } else {
            break;
        }
    }
    let has_word_end = text[end..]
        .chars()
        .next()
        .is_none_or(|character| !is_word_character(character));
    (count >= 2 && has_word_end).then_some(end)
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

// ---------------------------------------------------------------------------
// Inline-code extraction (voice / pitch codes in queue items)
// ---------------------------------------------------------------------------

pub(crate) fn extract_regex_group(re: &regex::Regex, text: &str) -> Option<String> {
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub fn extract_voice(codes: &str) -> Option<String> {
    extract_regex_group(&VOICE_RE, codes)
}

pub fn extract_pitch(codes: &str) -> Option<f32> {
    extract_regex_group(&PITCH_RE, codes).and_then(|s| s.parse().ok())
}

pub fn extract_logical_voice(codes: &str) -> Option<String> {
    extract_regex_group(&LOGICAL_VOICE_RE, codes)
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn home_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
}

/// Decode the double-quoted Tcl word emitted by Emacsvox for a resource path.
///
/// Unquoted and brace-delimited command arguments have already been normalized
/// by the protocol parser and are returned unchanged. Backslashes in unquoted
/// paths must remain untouched so native Windows paths continue to work.
fn decode_tcl_resource_word(argument: &str) -> Result<String, String> {
    if !argument.starts_with('"') {
        return Ok(argument.to_string());
    }
    if argument.len() < 2 || !argument.ends_with('"') {
        return Err("unterminated double-quoted Tcl resource path".to_string());
    }

    let mut decoded = String::with_capacity(argument.len().saturating_sub(2));
    let mut chars = argument[1..argument.len() - 1].chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        let escaped = chars
            .next()
            .ok_or_else(|| "trailing backslash in Tcl resource path".to_string())?;
        match escaped {
            '\\' => decoded.push('\\'),
            '"' => decoded.push('"'),
            '$' => decoded.push('$'),
            '[' => decoded.push('['),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'u' => {
                let mut value = 0_u32;
                for _ in 0..4 {
                    let digit = chars.next().ok_or_else(|| {
                        "incomplete \\u escape in Tcl resource path".to_string()
                    })?;
                    value = value * 16
                        + digit.to_digit(16).ok_or_else(|| {
                            "invalid \\u escape in Tcl resource path".to_string()
                        })?;
                }
                let character = char::from_u32(value).ok_or_else(|| {
                    "invalid Unicode scalar in Tcl resource path".to_string()
                })?;
                if character == '\0' {
                    return Err("Tcl resource path cannot contain NUL".to_string());
                }
                decoded.push(character);
            }
            other => {
                return Err(format!(
                    "unsupported Tcl escape \\{other} in resource path"
                ));
            }
        }
    }

    Ok(decoded)
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return PathBuf::from(home).join(rest);
        }
    } else if path == "~" {
        if let Some(home) = home_dir() {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

/// Parse one resource path argument and expand a leading home-directory marker.
pub fn parse_resource_path(argument: &str) -> Result<PathBuf, String> {
    decode_tcl_resource_word(argument).map(|path| expand_tilde(&path))
}

// ---------------------------------------------------------------------------
// Rate normalisation
// ---------------------------------------------------------------------------

/// Normalise an Emacspeak speech rate to the internal [0.0, 2.0] float scale.
///
/// Emacspeak sends integer rates (e.g. 50 = normal, 100 = fast).  Values above
/// 1.0 are divided by 100.  The upper bound is 2.0 so that engines supporting
/// it (piper) can reach ~10× speed; engines that don't (espeak, native) clamp
/// internally.
pub fn normalize_rate(rate: f32) -> f32 {
    let r = if rate > 1.0 { rate / 100.0 } else { rate };
    r.clamp(0.0, 2.0)
}

/// Per-chunk leading/trailing silence padding scaled by speech rate.
///
/// Slower rates get slightly more padding to avoid clipping.
pub fn rate_scaled_padding(rate: f32) -> f32 {
    let rate = rate.clamp(0.0, 1.0);
    0.002 + 0.013 * (1.0 - rate)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_space_before_uppercase() {
        assert_eq!(insert_space_before_uppercase("helloWorld"), "hello World");
        assert_eq!(insert_space_before_uppercase("CamelCaseIdentifier"), "Camel Case Identifier");
        assert_eq!(insert_space_before_uppercase("HTTPServer"), "HTTPServer");
        assert_eq!(insert_space_before_uppercase("isHTTPMethod"), "is HTTPMethod");
        assert_eq!(insert_space_before_uppercase("lowercase"), "lowercase");
    }

    #[test]
    fn capitalization_preparation_distinguishes_caps_and_all_caps() {
        let state = TtsState {
            punctuation_level: PunctuationLevel::None,
            split_caps: false,
            allcaps_beep: true,
            ..TtsState::default()
        };
        let prepared = prepare_speech_text("Hello camelCase ABC A1", &state);

        assert_eq!(prepared.text, "Hello camelCase abc a1");
        assert_eq!(
            prepared
                .capitalization_tones
                .iter()
                .map(|tone| (tone.text_offset, tone.frequency_hz))
                .collect::<Vec<_>>(),
            vec![(0, 440.0), (11, 440.0), (16, 1300.0), (20, 1300.0)]
        );
        assert!(prepared
            .capitalization_tones
            .iter()
            .all(|tone| tone.duration_ms == 20));
    }

    #[test]
    fn prepared_chunking_rebases_capitalization_offsets() {
        let prepared = PreparedSpeechText {
            text: "One two  Three four".to_owned(),
            capitalization_tones: vec![
                CapitalizationTone {
                    id: "first".to_owned(),
                    text_offset: 0,
                    frequency_hz: 440.0,
                    duration_ms: 20,
                },
                CapitalizationTone {
                    id: "third".to_owned(),
                    text_offset: 9,
                    frequency_hz: 440.0,
                    duration_ms: 20,
                },
            ],
        };

        let chunks = chunk_prepared_speech(prepared, 2);

        assert_eq!(chunks[0].text, "One two");
        assert_eq!(chunks[0].capitalization_tones[0].text_offset, 0);
        assert_eq!(chunks[1].text, "Three four");
        assert_eq!(chunks[1].capitalization_tones[0].text_offset, 0);
    }

    #[test]
    fn test_extract_voice() {
        assert_eq!(
            extract_voice("[{voice en-US:Samantha}]"),
            Some("en-US:Samantha".to_string())
        );
        assert_eq!(
            extract_voice("[{voice en-GB:Daniel}]"),
            Some("en-GB:Daniel".to_string())
        );
        assert_eq!(extract_voice("no voice here"), None);
    }

    #[test]
    fn test_extract_pitch() {
        assert_eq!(extract_pitch("[[pitch 1.5]]"), Some(1.5f32));
        assert_eq!(extract_pitch("[[pitch 0.8]]"), Some(0.8f32));
        assert_eq!(extract_pitch("no pitch here"), None);
    }

    #[test]
    fn test_extract_logical_voice() {
        let codes = "[[logical_voice source-code]] [[pitch 1.2]]";
        assert_eq!(extract_logical_voice(codes).as_deref(), Some("source-code"));
        assert_eq!(extract_logical_voice("[[logical_voice invalid voice]]"), None);
        assert_eq!(extract_logical_voice("[[logical_voice ../invalid]]"), None);
    }

    #[test]
    fn test_apply_punctuation_none() {
        assert_eq!(apply_punctuation("hello $100", PunctuationLevel::None), "hello  dollar 100");
    }

    #[test]
    fn test_apply_punctuation_some() {
        assert_eq!(apply_punctuation("a+b", PunctuationLevel::Some), "a plus b");
    }

    #[test]
    fn test_apply_punctuation_all() {
        assert_eq!(apply_punctuation("a.b", PunctuationLevel::All), "a dot b");
    }

    #[test]
    fn test_normalize_rate_normal() {
        assert!((normalize_rate(0.5) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_normalize_rate_integer_scale() {
        assert!((normalize_rate(50.0) - 0.5).abs() < 0.001);
        assert!((normalize_rate(100.0) - 1.0).abs() < 0.001);
        assert!((normalize_rate(150.0) - 1.5).abs() < 0.001);
        assert!((normalize_rate(200.0) - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_rate_clamp() {
        assert_eq!(normalize_rate(-1.0), 0.0);
        assert!((normalize_rate(300.0) - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        assert_eq!(expand_tilde("/absolute/path"), PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_parse_resource_path_decodes_emacsvox_tcl_word() {
        let encoded =
            r#""/tmp/cue space {brace} quote\" back\\slash; dollar\$ \[command] λ\n.ogg""#;
        let expected = "/tmp/cue space {brace} quote\" back\\slash; dollar$ [command] λ\n.ogg";
        assert_eq!(parse_resource_path(encoded).unwrap(), PathBuf::from(expected));
    }

    #[test]
    fn test_parse_resource_path_preserves_unquoted_windows_backslashes() {
        let path = r"C:\Users\Bart\sounds\complete.ogg";
        assert_eq!(parse_resource_path(path).unwrap(), PathBuf::from(path));

        let encoded = r#""C:\\Users\\Bart\\sounds\\complete.ogg""#;
        assert_eq!(parse_resource_path(encoded).unwrap(), PathBuf::from(path));
    }

    #[test]
    fn test_parse_resource_path_rejects_malformed_tcl_word() {
        assert!(parse_resource_path(r#""unterminated"#).is_err());
        assert!(parse_resource_path(r#""bad\qescape""#).is_err());
        assert!(parse_resource_path(r#""nul\u0000path""#).is_err());
    }

    #[test]
    fn test_rate_scaled_padding() {
        let slow = rate_scaled_padding(0.0);
        let fast = rate_scaled_padding(1.0);
        assert!(slow > fast);
        assert!(slow <= 0.02);
        assert!(fast >= 0.001);
    }
}
