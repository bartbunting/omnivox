//! Versioned Base64-JSON events for marker-aware playback dispatches.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{AcssDimension, PhysicalVoiceId, PostSynthesisDimension};
use crate::voice_choices::AudioChoiceIdentity;
use crate::{AnchorResolution, SynthesisMarker};

/// Current marker event protocol version.
pub const MARKER_PROTOCOL_VERSION: u32 = 1;
/// Marker/event protocol version adding playback-bound semantic actions.
pub const TIMELINE_EVENT_PROTOCOL_VERSION: u32 = 2;
/// Marker protocol carrying the actual layered choice at first consumed frame.
pub const VOICE_CHOICE_EVENT_PROTOCOL_VERSION: u32 = 3;
pub const MAX_VOICE_CHOICE_RECEIPT_BYTES: usize = 32 * 1024;
pub const MAX_V3_MARKER_RECORD_BYTES: usize = 512 * 1024;
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
pub const MAX_MARKER_EVENT_ENCODED_BYTES: usize = (MAX_MARKER_EVENT_PAYLOAD_BYTES / 3) * 4 + 8;

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
    VoiceChoiceApplied(VoiceChoiceApplied),
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
    /// Synthesis resolved one requested presentation action position.
    TimelineActionResolved {
        utterance_id: u64,
        action_id: String,
        resolution: AnchorResolution,
    },
    /// Requested style dimensions omitted on the realized route.
    TimelineStyleDegraded {
        utterance_id: u64,
        degraded_acss: Vec<AcssDimension>,
        degraded_effects: Vec<PostSynthesisDimension>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceChoiceApplied {
    pub utterance_id: u64,
    pub span_id: u64,
    pub registry_generation: u64,
    pub logical_voice_id: String,
    pub choice: AudioChoiceIdentity,
}

/// Encoding or decoding failure for one marker event.
#[derive(Debug, Error)]
pub enum MarkerProtocolError {
    #[error("marker event exceeds the {MAX_MARKER_EVENT_PAYLOAD_BYTES}-byte limit")]
    PayloadTooLarge,

    #[error("voice choice receipt exceeds the {MAX_VOICE_CHOICE_RECEIPT_BYTES}-byte limit")]
    ChoiceReceiptTooLarge,

    #[error("version 3 marker record exceeds the {MAX_V3_MARKER_RECORD_BYTES}-byte line limit")]
    RecordTooLarge,

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
    let encoded = encode_json(event)?;
    validate_v3_size(event, &encoded)?;
    Ok(encoded)
}

/// Decode and bound one marker event field.
pub fn decode_marker_event(payload: &str) -> Result<MarkerEventEnvelope, MarkerProtocolError> {
    let event = decode_json(payload)?;
    validate_event(&event)?;
    validate_v3_size(&event, payload)?;
    if event.protocol_version == VOICE_CHOICE_EVENT_PROTOCOL_VERSION {
        let bytes = STANDARD
            .decode(payload)
            .map_err(MarkerProtocolError::InvalidBase64)?;
        serde_json::from_slice::<crate::control::DuplicateFreeJson>(&bytes)
            .map_err(MarkerProtocolError::InvalidJson)?;
    }
    Ok(event)
}

fn validate_v3_size(event: &MarkerEventEnvelope, payload: &str) -> Result<(), MarkerProtocolError> {
    if event.protocol_version != VOICE_CHOICE_EVENT_PROTOCOL_VERSION {
        return Ok(());
    }
    // Include prefix, separator and newline in the existing remote line budget.
    if payload.len() + MARKER_EVENT_PREFIX.len() + 2 > MAX_V3_MARKER_RECORD_BYTES {
        return Err(MarkerProtocolError::RecordTooLarge);
    }
    let padding = payload
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    let decoded = payload.len() / 4 * 3 - padding;
    if matches!(event.event, MarkerEvent::VoiceChoiceApplied(_))
        && decoded > MAX_VOICE_CHOICE_RECEIPT_BYTES
    {
        return Err(MarkerProtocolError::ChoiceReceiptTooLarge);
    }
    Ok(())
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
    if event.protocol_version == VOICE_CHOICE_EVENT_PROTOCOL_VERSION
        && (event.dispatch_id == 0 || event.sequence == 0)
    {
        return Err(MarkerProtocolError::InvalidEnvelope(
            "dispatch ID and sequence must be positive".to_owned(),
        ));
    }
    match &event.event {
        MarkerEvent::VoiceChoiceApplied(choice) => {
            if event.protocol_version != VOICE_CHOICE_EVENT_PROTOCOL_VERSION {
                return Err(MarkerProtocolError::InvalidEnvelope(
                    "voice choice receipts require protocol version 3".to_owned(),
                ));
            }
            if choice.utterance_id == 0 || choice.span_id == 0 || choice.registry_generation == 0 {
                return Err(MarkerProtocolError::InvalidEnvelope(
                    "choice receipt identities must be positive".to_owned(),
                ));
            }
            crate::timeline_protocol::validate_id(&choice.logical_voice_id, "logical voice")
                .map_err(|error| MarkerProtocolError::InvalidEnvelope(error.to_string()))?;
        }
        MarkerEvent::SemanticEventReached { action_id, .. }
        | MarkerEvent::TimelineActionResolved { action_id, .. } => {
            if !matches!(
                event.protocol_version,
                TIMELINE_EVENT_PROTOCOL_VERSION | VOICE_CHOICE_EVENT_PROTOCOL_VERSION
            ) {
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
        MarkerEvent::TimelineStyleDegraded { .. }
            if !matches!(
                event.protocol_version,
                TIMELINE_EVENT_PROTOCOL_VERSION | VOICE_CHOICE_EVENT_PROTOCOL_VERSION
            ) =>
        {
            return Err(MarkerProtocolError::InvalidEnvelope(
                "timeline style diagnostics require protocol version 2".to_owned(),
            ));
        }
        _ if event.protocol_version != MARKER_PROTOCOL_VERSION
            && event.protocol_version != TIMELINE_EVENT_PROTOCOL_VERSION
            && event.protocol_version != VOICE_CHOICE_EVENT_PROTOCOL_VERSION =>
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

    fn choice_fixture() -> MarkerEventEnvelope {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/protocol-fixtures/voice-choice-tuning.json"
        ))
        .unwrap();
        serde_json::from_value(value["messages"]["playback_receipt"].clone()).unwrap()
    }

    #[test]
    fn choice_receipt_matches_independent_fixture_and_requires_new_version() {
        let mut event = choice_fixture();
        assert_eq!(
            decode_marker_event(&encode_marker_event(&event).unwrap()).unwrap(),
            event
        );
        for version in [MARKER_PROTOCOL_VERSION, TIMELINE_EVENT_PROTOCOL_VERSION] {
            event.protocol_version = version;
            assert!(encode_marker_event(&event).is_err());
        }
    }

    #[test]
    fn choice_receipt_rejects_missing_unknown_duplicate_and_zero_identity_fields() {
        let value = serde_json::to_value(choice_fixture()).unwrap();
        for field in [
            "utterance_id",
            "span_id",
            "registry_generation",
            "logical_voice_id",
            "choice",
        ] {
            let mut bad = value.clone();
            bad.as_object_mut().unwrap().remove(field);
            assert!(
                decode_marker_event(&STANDARD.encode(bad.to_string())).is_err(),
                "{field}"
            );
        }
        for field in [
            "utterance_id",
            "span_id",
            "registry_generation",
            "dispatch_id",
            "sequence",
        ] {
            let mut bad = value.clone();
            bad[field] = serde_json::json!(0);
            assert!(
                decode_marker_event(&STANDARD.encode(bad.to_string())).is_err(),
                "{field}"
            );
        }
        let mut bad = value.clone();
        bad["extra"] = serde_json::json!(true);
        assert!(decode_marker_event(&STANDARD.encode(bad.to_string())).is_err());
        let raw = value
            .to_string()
            .replacen("\"sequence\":2", "\"sequence\":2,\"sequence\":3", 1);
        assert!(decode_marker_event(&STANDARD.encode(raw)).is_err());
    }

    #[test]
    fn marker_v3_bounds_receipts_and_all_encoded_lines_without_changing_old_limits() {
        let mut receipt = choice_fixture();
        let MarkerEvent::VoiceChoiceApplied(choice) = &mut receipt.event else {
            unreachable!()
        };
        choice.choice.realized.voice_id = "x".repeat(MAX_VOICE_CHOICE_RECEIPT_BYTES);
        assert!(matches!(
            encode_marker_event(&receipt),
            Err(MarkerProtocolError::ChoiceReceiptTooLarge)
        ));
        let raw = STANDARD.encode(serde_json::to_vec(&receipt).unwrap());
        assert!(matches!(
            decode_marker_event(&raw),
            Err(MarkerProtocolError::ChoiceReceiptTooLarge)
        ));

        let mut started = MarkerEventEnvelope {
            protocol_version: VOICE_CHOICE_EVENT_PROTOCOL_VERSION,
            dispatch_id: 91,
            sequence: 1,
            event: MarkerEvent::UtteranceStarted {
                utterance_id: 1,
                text: "\u{1}".repeat(70_000),
                engine_id: "eloquence".to_owned(),
                actual_voice: Some(PhysicalVoiceId::new("eloquence", "Reed")),
                logical_voice_id: Some("bolden".to_owned()),
                sample_rate: 44100,
                frame_count: u64::MAX,
            },
        };
        assert!(matches!(
            encode_marker_event(&started),
            Err(MarkerProtocolError::RecordTooLarge)
        ));
        let raw = STANDARD.encode(serde_json::to_vec(&started).unwrap());
        assert!(matches!(
            decode_marker_event(&raw),
            Err(MarkerProtocolError::RecordTooLarge)
        ));
        started.protocol_version = TIMELINE_EVENT_PROTOCOL_VERSION;
        assert!(encode_marker_event(&started).is_ok());
        assert!(decode_marker_event(&encode_marker_event(&started).unwrap()).is_ok());

        let semantic = MarkerEventEnvelope {
            protocol_version: VOICE_CHOICE_EVENT_PROTOCOL_VERSION,
            dispatch_id: 91,
            sequence: 3,
            event: MarkerEvent::SemanticEventReached {
                utterance_id: 1,
                action_id: "heading".to_owned(),
            },
        };
        assert!(encode_marker_event(&semantic).is_ok());
    }

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

    #[test]
    fn timeline_diagnostics_round_trip_in_v2() {
        let resolution = MarkerEventEnvelope {
            protocol_version: TIMELINE_EVENT_PROTOCOL_VERSION,
            dispatch_id: 9,
            sequence: 2,
            event: MarkerEvent::TimelineActionResolved {
                utterance_id: 1,
                action_id: "open-cue".to_owned(),
                resolution: AnchorResolution::WordBoundary,
            },
        };
        let degradation = MarkerEventEnvelope {
            protocol_version: TIMELINE_EVENT_PROTOCOL_VERSION,
            dispatch_id: 9,
            sequence: 3,
            event: MarkerEvent::TimelineStyleDegraded {
                utterance_id: 1,
                degraded_acss: vec![AcssDimension::Richness],
                degraded_effects: vec![PostSynthesisDimension::Echo],
            },
        };

        for event in [resolution, degradation] {
            assert_eq!(
                decode_marker_event(&encode_marker_event(&event).unwrap()).unwrap(),
                event
            );
        }
    }
}
