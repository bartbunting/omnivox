//! Versioned Base64-JSON events for marker-aware playback dispatches.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::PhysicalVoiceId;
use crate::SynthesisMarker;

/// Current marker event protocol version.
pub const MARKER_PROTOCOL_VERSION: u32 = 1;
/// Marker/event protocol version adding playback-bound semantic actions.
pub const TIMELINE_EVENT_PROTOCOL_VERSION: u32 = 2;
/// Maximum UTF-8 size of an opaque semantic action identifier.
pub const MAX_SEMANTIC_ACTION_ID_BYTES: usize = 128;

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
    /// The playback source reached an opaque presentation-timeline action.
    SemanticEventReached {
        utterance_id: u64,
        action_id: String,
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

    #[error("invalid marker event envelope: {0}")]
    InvalidEnvelope(String),
}

/// Encode one marker event as an unwrapped Base64 field.
pub fn encode_marker_event(event: &MarkerEventEnvelope) -> Result<String, MarkerProtocolError> {
    validate_event(event)?;
    encode_json(event)
}

/// Decode and bound one marker event field.
pub fn decode_marker_event(payload: &str) -> Result<MarkerEventEnvelope, MarkerProtocolError> {
    let event = decode_json(payload)?;
    validate_event(&event)?;
    Ok(event)
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

fn validate_event(event: &MarkerEventEnvelope) -> Result<(), MarkerProtocolError> {
    match &event.event {
        MarkerEvent::SemanticEventReached { action_id, .. } => {
            if event.protocol_version != TIMELINE_EVENT_PROTOCOL_VERSION {
                return Err(MarkerProtocolError::InvalidEnvelope(
                    "semantic events require protocol version 2".to_owned(),
                ));
            }
            if action_id.is_empty() || action_id.len() > MAX_SEMANTIC_ACTION_ID_BYTES {
                return Err(MarkerProtocolError::InvalidEnvelope(format!(
                    "semantic action ID must contain 1 to {MAX_SEMANTIC_ACTION_ID_BYTES} UTF-8 bytes"
                )));
            }
        }
        _ if event.protocol_version != MARKER_PROTOCOL_VERSION
            && event.protocol_version != TIMELINE_EVENT_PROTOCOL_VERSION =>
        {
            return Err(MarkerProtocolError::InvalidEnvelope(format!(
                "unsupported protocol version {}",
                event.protocol_version
            )));
        }
        _ => {}
    }
    Ok(())
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

    #[test]
    fn semantic_events_require_v2_and_a_bounded_opaque_id() {
        let semantic = |protocol_version, action_id: String| MarkerEventEnvelope {
            protocol_version,
            dispatch_id: 9,
            sequence: 3,
            event: MarkerEvent::SemanticEventReached {
                utterance_id: 1,
                action_id,
            },
        };
        let valid = semantic(TIMELINE_EVENT_PROTOCOL_VERSION, "object-entered".to_owned());
        assert_eq!(
            decode_marker_event(&encode_marker_event(&valid).unwrap()).unwrap(),
            valid
        );

        let wrong_version = semantic(MARKER_PROTOCOL_VERSION, "event".to_owned());
        assert!(matches!(
            encode_marker_event(&wrong_version),
            Err(MarkerProtocolError::InvalidEnvelope(_))
        ));
        let oversized = semantic(
            TIMELINE_EVENT_PROTOCOL_VERSION,
            "x".repeat(MAX_SEMANTIC_ACTION_ID_BYTES + 1),
        );
        assert!(matches!(
            encode_marker_event(&oversized),
            Err(MarkerProtocolError::InvalidEnvelope(_))
        ));
    }
}
