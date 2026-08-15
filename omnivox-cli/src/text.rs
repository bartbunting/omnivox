//! Text processing: punctuation expansion, CamelCase splitting, chunking, rate mapping.

use omnivox_core::{state::PunctuationLevel, TtsState};
use once_cell::sync::Lazy;
use std::path::PathBuf;

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
    /// UTF-8 range in the complete prepared speech text.
    pub source_start: u32,
    pub source_end: u32,
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
            source_start: 0,
            source_end: prepared.text.len() as u32,
            text: prepared.text,
            capitalization_tones: prepared.capitalization_tones,
        }];
    }

    let mut chunks = Vec::new();
    let mut first_word = 0;
    while first_word < spans.len() {
        let hard_end = (first_word + max_words).min(spans.len());
        let word_end = if hard_end == spans.len() {
            hard_end
        } else {
            preferred_chunk_end(&prepared.text, &spans, first_word, hard_end)
        };
        let start = spans[first_word].0;
        let end = spans[word_end - 1].1;
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
        chunks.push(PreparedSpeechChunk {
            text: prepared.text[start..end].to_owned(),
            capitalization_tones,
            source_start: start as u32,
            source_end: end as u32,
        });
        first_word = word_end;
    }
    chunks
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkBoundary {
    Sentence,
    Clause,
}

fn preferred_chunk_end(
    text: &str,
    spans: &[(usize, usize)],
    first_word: usize,
    hard_end: usize,
) -> usize {
    let mut sentence = None;
    let mut clause = None;
    for word_index in first_word..hard_end {
        match boundary_after_word(text, spans, word_index) {
            Some(ChunkBoundary::Sentence) => sentence = Some(word_index + 1),
            Some(ChunkBoundary::Clause) => clause = Some(word_index + 1),
            None => {}
        }
    }
    sentence.or(clause).unwrap_or(hard_end)
}

fn boundary_after_word(
    text: &str,
    spans: &[(usize, usize)],
    word_index: usize,
) -> Option<ChunkBoundary> {
    let (_, end) = spans[word_index];
    if let Some((next_start, _)) = spans.get(word_index + 1) {
        if text[end..*next_start]
            .chars()
            .any(|character| character == '\r' || character == '\n')
        {
            return Some(ChunkBoundary::Sentence);
        }
    }

    let (start, end) = spans[word_index];
    let last = text[start..end]
        .trim_end_matches(is_boundary_closer)
        .chars()
        .next_back()?;
    if matches!(last, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
        Some(ChunkBoundary::Sentence)
    } else if matches!(last, ',' | ';' | ':' | '—' | '–') {
        Some(ChunkBoundary::Clause)
    } else {
        None
    }
}

fn is_boundary_closer(character: char) -> bool {
    matches!(character, '\'' | '"' | '’' | '”' | ')' | ']' | '}')
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
        let replacement = punctuation_replacement(ch, level);

        match replacement {
            Some(spoken) => result.push_str(spoken),
            None => result.push(ch),
        }
    }

    result
}

fn punctuation_replacement(character: char, level: PunctuationLevel) -> Option<&'static str> {
    match level {
        PunctuationLevel::None => match character {
            '$' => Some(" dollar "),
            '%' => Some(" percent "),
            _ => None,
        },
        PunctuationLevel::Some => match character {
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
        PunctuationLevel::All => match character {
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
    }
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

/// Apply speech preprocessing without inferring capitalization presentation.
///
/// Semantic presentations carry capitalization actions explicitly.  The
/// legacy text path therefore preserves case and leaves the tone list empty.
pub fn prepare_speech_text(text: &str, state: &TtsState) -> PreparedSpeechText {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    PreparedSpeechText {
        text: processed,
        capitalization_tones: Vec::new(),
    }
}

/// Apply speech preprocessing and remap caller-requested UTF-8 boundaries.
///
/// The returned offsets are in the prepared text and retain input order.
/// Callers must supply valid UTF-8 boundaries in `text`.
pub fn prepare_speech_text_with_offsets(
    text: &str,
    state: &TtsState,
    offsets: &[u32],
) -> (PreparedSpeechText, Vec<u32>) {
    debug_assert!(offsets.iter().all(|offset| {
        let offset = *offset as usize;
        offset <= text.len() && text.is_char_boundary(offset)
    }));
    let (punctuated, offsets) =
        apply_punctuation_with_offsets(text, state.punctuation_level, offsets);
    let (split, offsets) = if state.split_caps {
        insert_space_before_uppercase_with_offsets(&punctuated, &offsets)
    } else {
        (punctuated, offsets)
    };
    (
        PreparedSpeechText {
            text: split,
            capitalization_tones: Vec::new(),
        },
        offsets,
    )
}

fn apply_punctuation_with_offsets(
    text: &str,
    level: PunctuationLevel,
    offsets: &[u32],
) -> (String, Vec<u32>) {
    let mut output = String::with_capacity(text.len());
    let mut mapped = vec![0_u32; offsets.len()];
    for (position, character) in text.char_indices() {
        record_offsets(offsets, position, output.len(), &mut mapped);
        if let Some(replacement) = punctuation_replacement(character, level) {
            output.push_str(replacement);
        } else {
            output.push(character);
        }
    }
    record_offsets(offsets, text.len(), output.len(), &mut mapped);
    (output, mapped)
}

fn insert_space_before_uppercase_with_offsets(text: &str, offsets: &[u32]) -> (String, Vec<u32>) {
    let mut output = String::with_capacity(text.len() * 2);
    let mut mapped = vec![0_u32; offsets.len()];
    let mut previous_lower = false;
    for (position, character) in text.char_indices() {
        if character.is_uppercase() && previous_lower {
            output.push(' ');
        }
        record_offsets(offsets, position, output.len(), &mut mapped);
        previous_lower = character.is_lowercase();
        output.push(character);
    }
    record_offsets(offsets, text.len(), output.len(), &mut mapped);
    (output, mapped)
}

fn record_offsets(offsets: &[u32], input: usize, output: usize, mapped: &mut [u32]) {
    for (index, offset) in offsets.iter().enumerate() {
        if *offset as usize == input {
            mapped[index] = output as u32;
        }
    }
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
                    let digit = chars
                        .next()
                        .ok_or_else(|| "incomplete \\u escape in Tcl resource path".to_string())?;
                    value = value * 16
                        + digit
                            .to_digit(16)
                            .ok_or_else(|| "invalid \\u escape in Tcl resource path".to_string())?;
                }
                let character = char::from_u32(value)
                    .ok_or_else(|| "invalid Unicode scalar in Tcl resource path".to_string())?;
                if character == '\0' {
                    return Err("Tcl resource path cannot contain NUL".to_string());
                }
                decoded.push(character);
            }
            other => {
                return Err(format!("unsupported Tcl escape \\{other} in resource path"));
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
        assert_eq!(
            insert_space_before_uppercase("CamelCaseIdentifier"),
            "Camel Case Identifier"
        );
        assert_eq!(insert_space_before_uppercase("HTTPServer"), "HTTPServer");
        assert_eq!(
            insert_space_before_uppercase("isHTTPMethod"),
            "is HTTPMethod"
        );
        assert_eq!(insert_space_before_uppercase("lowercase"), "lowercase");
    }

    #[test]
    fn queued_preparation_does_not_infer_capitalization_actions() {
        let state = TtsState {
            punctuation_level: PunctuationLevel::None,
            split_caps: false,
            ..TtsState::default()
        };
        let prepared = prepare_speech_text("Hello camelCase ABC A1", &state);

        assert_eq!(prepared.text, "Hello camelCase ABC A1");
        assert!(prepared.capitalization_tones.is_empty());
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
        assert_eq!((chunks[0].source_start, chunks[0].source_end), (0, 7));
        assert_eq!(chunks[0].capitalization_tones[0].text_offset, 0);
        assert_eq!(chunks[1].text, "Three four");
        assert_eq!((chunks[1].source_start, chunks[1].source_end), (9, 19));
        assert_eq!(chunks[1].capitalization_tones[0].text_offset, 0);
    }

    #[test]
    fn prepared_chunking_prefers_sentences_then_clauses() {
        let prepared = PreparedSpeechText {
            text: "One two three. Four five six seven, eight nine ten eleven twelve".to_owned(),
            capitalization_tones: Vec::new(),
        };

        let chunks = chunk_prepared_speech(prepared, 5);

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "One two three.",
                "Four five six seven,",
                "eight nine ten eleven twelve"
            ]
        );
        assert!(chunks
            .iter()
            .all(|chunk| word_spans(&chunk.text).len() <= 5));
    }

    #[test]
    fn prepared_chunking_treats_newlines_and_unicode_closers_as_sentences() {
        let prepared = PreparedSpeechText {
            text: "First line\nSecond line continues here. “Really?” Third tail words".to_owned(),
            capitalization_tones: Vec::new(),
        };

        let chunks = chunk_prepared_speech(prepared, 5);

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "First line",
                "Second line continues here. “Really?”",
                "Third tail words"
            ]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (chunk.source_start, chunk.source_end))
                .collect::<Vec<_>>(),
            vec![(0, 10), (11, 52), (53, 69)]
        );

        let quoted = chunk_prepared_speech(
            PreparedSpeechText {
                text: "“Really?” one two three four five".to_owned(),
                capitalization_tones: Vec::new(),
            },
            4,
        );
        assert_eq!(quoted[0].text, "“Really?”");
    }

    #[test]
    fn prepared_chunking_falls_back_to_the_hard_word_limit() {
        let prepared = PreparedSpeechText {
            text: "one two three four five six".to_owned(),
            capitalization_tones: Vec::new(),
        };

        let chunks = chunk_prepared_speech(prepared, 2);

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<Vec<_>>(),
            vec!["one two", "three four", "five six"]
        );
    }

    #[test]
    fn speech_preprocessing_remaps_requested_utf8_boundaries() {
        let state = TtsState {
            punctuation_level: PunctuationLevel::None,
            split_caps: true,
            ..TtsState::default()
        };
        let input = "cash$Value ABC";
        let (prepared, offsets) =
            prepare_speech_text_with_offsets(input, &state, &[4, 5, 11, input.len() as u32]);

        assert_eq!(prepared.text, "cash dollar Value ABC");
        assert!(prepared.text[offsets[0] as usize..].starts_with(" dollar "));
        assert!(prepared.text[offsets[1] as usize..].starts_with("Value"));
        assert!(prepared.text[offsets[2] as usize..].starts_with("ABC"));
        assert_eq!(offsets[3] as usize, prepared.text.len());
        assert_eq!(
            prepared,
            prepare_speech_text(input, &state),
            "tracking offsets must not change speech preparation"
        );
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
        assert_eq!(
            extract_logical_voice("[[logical_voice invalid voice]]"),
            None
        );
        assert_eq!(extract_logical_voice("[[logical_voice ../invalid]]"), None);
    }

    #[test]
    fn test_apply_punctuation_none() {
        assert_eq!(
            apply_punctuation("hello $100", PunctuationLevel::None),
            "hello  dollar 100"
        );
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
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
    }

    #[test]
    fn test_parse_resource_path_decodes_emacsvox_tcl_word() {
        let encoded =
            r#""/tmp/cue space {brace} quote\" back\\slash; dollar\$ \[command] λ\n.ogg""#;
        let expected = "/tmp/cue space {brace} quote\" back\\slash; dollar$ [command] λ\n.ogg";
        assert_eq!(
            parse_resource_path(encoded).unwrap(),
            PathBuf::from(expected)
        );
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
