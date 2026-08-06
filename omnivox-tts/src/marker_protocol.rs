//! Versioned Base64-JSON events for marker-aware playback dispatches.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::PhysicalVoiceId;
use crate::SynthesisMarker;

/// Current marker event protocol version.
pub const MARKER_PROTOCOL_VERSION: u32 = 1;

/// Prefix used for marker playback events on stdout.
pub const MARKER_EVENT_PREFIX: &str = "__EMACSVOX_MARKER__";

/// Maximum decoded marker event size.
///
/// This accommodates the bounded 256 KiB presentation text after worst-case
/// JSON escaping while still bounding server output and client allocation.
pub const MAX_MARKER_EVENT_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

/// Conservative maximum Base64 size for the decoded event bound.
pub const MAX_MARKER_EVENT_ENCODED_BYTES: usize =
    (MAX_MARKER_EVENT_PAYLOAD_BYTES / 3) * 4 + 8;

/// One event emitted by a marker-aware dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerEventEnvelope {
    pub protocol_version: u32,
    pub dispatch_id: u64,
    /// One-based event sequence within the dispatch.
    pub sequence: u64,
    #[serde(flatten)]
    pub event: MarkerEvent,
}

/// Playback events available to marker-aware clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MarkerEvent {
    /// The playback source reached the first frame of one synthesized chunk.
    UtteranceStarted {
        utterance_id: u64,
        text: String,
        engine_id: String,
        actual_voice: Option<PhysicalVoiceId>,
        logical_voice_id: Option<String>,
        sample_rate: u32,
        frame_count: u64,
    },
    /// The playback source reached an engine-provided marker.
    MarkerReached {
        utterance_id: u64,
        marker: SynthesisMarker,
    },
}

/// Encoding or decoding failure for one marker event.
#[derive(Debug, Error)]
pub enum MarkerProtocolError {
    #[error("marker event exceeds the {MAX_MARKER_EVENT_PAYLOAD_BYTES}-byte limit")]
    PayloadTooLarge,

    #[error("marker event is not valid Base64: {0}")]
    InvalidBase64(#[source] base64::DecodeError),

    #[error("marker event is not valid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
}

/// Encode one marker event as an unwrapped Base64 field.
pub fn encode_marker_event(event: &MarkerEventEnvelope) -> Result<String, MarkerProtocolError> {
    encode_json(event)
}

/// Decode and bound one marker event field.
pub fn decode_marker_event(payload: &str) -> Result<MarkerEventEnvelope, MarkerProtocolError> {
    decode_json(payload)
}

/// Format one newline-free marker event record.
pub fn format_marker_event(event: &MarkerEventEnvelope) -> Result<String, MarkerProtocolError> {
    Ok(format!(
        "{} {}",
        MARKER_EVENT_PREFIX,
        encode_marker_event(event)?
    ))
}

fn encode_json<T: Serialize>(value: &T) -> Result<String, MarkerProtocolError> {
    let json = serde_json::to_vec(value).map_err(MarkerProtocolError::InvalidJson)?;
    if json.len() > MAX_MARKER_EVENT_PAYLOAD_BYTES {
        return Err(MarkerProtocolError::PayloadTooLarge);
    }
    Ok(STANDARD.encode(json))
}

fn decode_json<T: DeserializeOwned>(payload: &str) -> Result<T, MarkerProtocolError> {
    if payload.len() > MAX_MARKER_EVENT_ENCODED_BYTES {
        return Err(MarkerProtocolError::PayloadTooLarge);
    }
    let json = STANDARD
        .decode(payload)
        .map_err(MarkerProtocolError::InvalidBase64)?;
    if json.len() > MAX_MARKER_EVENT_PAYLOAD_BYTES {
        return Err(MarkerProtocolError::PayloadTooLarge);
    }
    serde_json::from_slice(&json).map_err(MarkerProtocolError::InvalidJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SynthesisMarkerKind;

    fn marker_event() -> MarkerEventEnvelope {
        MarkerEventEnvelope {
            protocol_version: MARKER_PROTOCOL_VERSION,
            dispatch_id: 73,
            sequence: 2,
            event: MarkerEvent::MarkerReached {
                utterance_id: 1,
                marker: SynthesisMarker {
                    kind: SynthesisMarkerKind::Word,
                    frame_offset: 4410,
                    text_start: Some(0),
                    text_length: Some(5),
                    value: Some("hello".to_owned()),
                },
            },
        }
    }

    #[test]
    fn marker_event_round_trip_is_base64_json() {
        let event = marker_event();
        let encoded = encode_marker_event(&event).unwrap();

        assert!(!encoded.contains('\n'));
        assert_eq!(decode_marker_event(&encoded).unwrap(), event);
    }

    #[test]
    fn utterance_event_preserves_route_and_unicode_text() {
        let event = MarkerEventEnvelope {
            protocol_version: MARKER_PROTOCOL_VERSION,
            dispatch_id: 9,
            sequence: 1,
            event: MarkerEvent::UtteranceStarted {
                utterance_id: 4,
                text: "café 日本".to_owned(),
                engine_id: "winrt".to_owned(),
                actual_voice: Some(PhysicalVoiceId::new("winrt", "voice:David")),
                logical_voice_id: Some("source-code".to_owned()),
                sample_rate: 44100,
                frame_count: 22050,
            },
        };

        let decoded = decode_marker_event(&encode_marker_event(&event).unwrap()).unwrap();

        assert_eq!(decoded, event);
    }

    #[test]
    fn formatted_event_has_one_machine_readable_prefix() {
        let record = format_marker_event(&marker_event()).unwrap();

        assert!(record.starts_with("__EMACSVOX_MARKER__ "));
        assert!(!record.contains('\n'));
    }

    #[test]
    fn encoded_payload_is_bounded_before_decoding() {
        let payload = "A".repeat(MAX_MARKER_EVENT_ENCODED_BYTES + 1);

        assert!(matches!(
            decode_marker_event(&payload),
            Err(MarkerProtocolError::PayloadTooLarge)
        ));
    }
}
