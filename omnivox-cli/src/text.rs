//! Text processing: punctuation expansion, CamelCase splitting, chunking, rate mapping.

use omnivox_core::{state::PunctuationLevel, TtsState};
use once_cell::sync::Lazy;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Compiled regexes (one-time cost)
// ---------------------------------------------------------------------------

pub(crate) static VOICE_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\{voice\s+([^\}]+)\}\]").expect("invalid voice regex"));

pub(crate) static PITCH_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"\[\[pitch\s+([^\]]+)\]\]").expect("invalid pitch regex"));

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split text into chunks of at most `max_words` words.
///
/// Keeps individual utterances small so the TTS engine produces single-buffer
/// output, enabling aggressive silence trimming and fast cancellation between
/// chunks.
pub fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return vec![text.to_string()];
    }
    words.chunks(max_words).map(|c| c.join(" ")).collect()
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

/// Apply punctuation expansion and optional CamelCase splitting to `text`.
pub fn preprocess_text(text: &str, state: &TtsState) -> String {
    let mut processed = apply_punctuation(text, state.punctuation_level);
    if state.split_caps {
        processed = insert_space_before_uppercase(&processed);
    }
    processed
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

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn home_dir() -> Option<std::ffi::OsString> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
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
    fn test_chunk_text_short() {
        let chunks = chunk_text("hello world", 15);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn test_chunk_text_long() {
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen";
        let chunks = chunk_text(text, 15);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].split_whitespace().count(), 15);
        assert_eq!(chunks[1].split_whitespace().count(), 1);
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
    fn test_rate_scaled_padding() {
        let slow = rate_scaled_padding(0.0);
        let fast = rate_scaled_padding(1.0);
        assert!(slow > fast);
        assert!(slow <= 0.02);
        assert!(fast >= 0.001);
    }
}
