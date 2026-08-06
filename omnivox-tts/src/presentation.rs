//! Bounded codec for replaceable Emacsvox presentation transactions.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use thiserror::Error;

/// Maximum decoded legacy-protocol script accepted in one transaction.
pub const MAX_PRESENTATION_PAYLOAD_BYTES: usize = 256 * 1024;

/// Conservative maximum encoded size for the decoded transaction bound.
pub const MAX_PRESENTATION_ENCODED_BYTES: usize =
    (MAX_PRESENTATION_PAYLOAD_BYTES / 3) * 4 + 8;

/// One decoded `emacsvox_tx GENERATION {BASE64}` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationFrame {
    pub generation: u64,
    pub script: String,
}

#[derive(Debug, Error)]
pub enum PresentationFrameError {
    #[error("transaction arguments must be GENERATION {{BASE64}}")]
    InvalidArguments,

    #[error("transaction generation must be a positive integer")]
    InvalidGeneration,

    #[error("transaction payload exceeds the {MAX_PRESENTATION_PAYLOAD_BYTES}-byte limit")]
    PayloadTooLarge,

    #[error("transaction payload is not valid Base64: {0}")]
    InvalidBase64(#[source] base64::DecodeError),

    #[error("transaction payload is not valid UTF-8: {0}")]
    InvalidUtf8(#[source] std::string::FromUtf8Error),
}

/// Encode a UTF-8 legacy-protocol script for an `emacsvox_tx` payload.
pub fn encode_presentation_script(script: &str) -> Result<String, PresentationFrameError> {
    if script.len() > MAX_PRESENTATION_PAYLOAD_BYTES {
        return Err(PresentationFrameError::PayloadTooLarge);
    }
    Ok(STANDARD.encode(script.as_bytes()))
}

/// Decode and bound the arguments captured after `emacsvox_tx`.
pub fn decode_presentation_frame(
    arguments: &str,
) -> Result<PresentationFrame, PresentationFrameError> {
    let arguments = arguments.trim();
    let split = arguments
        .find(char::is_whitespace)
        .ok_or(PresentationFrameError::InvalidArguments)?;
    let generation = arguments[..split]
        .parse::<u64>()
        .ok()
        .filter(|generation| *generation > 0)
        .ok_or(PresentationFrameError::InvalidGeneration)?;
    let encoded_word = arguments[split..].trim();
    let encoded = encoded_word
        .strip_prefix('{')
        .and_then(|word| word.strip_suffix('}'))
        .filter(|payload| !payload.contains(['{', '}', ' ', '\t', '\r', '\n']))
        .ok_or(PresentationFrameError::InvalidArguments)?;
    if encoded.len() > MAX_PRESENTATION_ENCODED_BYTES {
        return Err(PresentationFrameError::PayloadTooLarge);
    }

    let payload = STANDARD
        .decode(encoded)
        .map_err(PresentationFrameError::InvalidBase64)?;
    if payload.len() > MAX_PRESENTATION_PAYLOAD_BYTES {
        return Err(PresentationFrameError::PayloadTooLarge);
    }
    let script = String::from_utf8(payload).map_err(PresentationFrameError::InvalidUtf8)?;

    Ok(PresentationFrame { generation, script })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_utf8_transaction() {
        let encoded = encode_presentation_script("q {café 日本 }\nd\n").unwrap();

        let frame = decode_presentation_frame(&format!("17 {{{encoded}}}")).unwrap();

        assert_eq!(frame.generation, 17);
        assert_eq!(frame.script, "q {café 日本 }\nd\n");
    }

    #[test]
    fn rejects_invalid_generation_and_word_shape() {
        assert!(matches!(
            decode_presentation_frame("0 {ZA==}"),
            Err(PresentationFrameError::InvalidGeneration)
        ));
        assert!(matches!(
            decode_presentation_frame("1 ZA=="),
            Err(PresentationFrameError::InvalidArguments)
        ));
    }

    #[test]
    fn bounds_encoded_payload_before_decoding() {
        let encoded = "A".repeat(MAX_PRESENTATION_ENCODED_BYTES + 1);

        assert!(matches!(
            decode_presentation_frame(&format!("1 {{{encoded}}}")),
            Err(PresentationFrameError::PayloadTooLarge)
        ));
    }
}
