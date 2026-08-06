//! Versioned Base64-JSON control protocol.
//!
//! Presentation transactions remain in the legacy line protocol. This channel
//! carries discovery, configuration, and diagnostic messages without making
//! native identifiers part of Tcl command syntax.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

/// Current control protocol version.
pub const CONTROL_PROTOCOL_VERSION: u32 = 1;

/// Maximum decoded JSON payload accepted from a client.
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 256 * 1024;

/// Conservative maximum Base64 size for the decoded payload bound.
pub const MAX_CONTROL_ENCODED_BYTES: usize = (MAX_CONTROL_PAYLOAD_BYTES / 3) * 4 + 8;

/// Prefix used for machine-readable server events on stdout.
pub const CONTROL_EVENT_PREFIX: &str = "__OMNIVOX_CONTROL__";

/// One versioned client request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRequestEnvelope {
    pub protocol_version: u32,
    pub request_id: u64,
    #[serde(flatten)]
    pub request: ControlRequest,
}

/// Requests implemented by the initial control channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlRequest {
    Capabilities,
}

/// One versioned server response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: Option<u64>,
    #[serde(flatten)]
    pub response: ControlResponse,
}

/// Response payloads emitted by the initial control channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlResponse {
    Capabilities {
        server_version: String,
        supported_protocol_versions: Vec<u32>,
        features: Vec<String>,
    },
    Error {
        code: ControlErrorCode,
        message: String,
    },
}

/// Stable machine-readable control error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode {
    MalformedRequest,
    UnsupportedVersion,
    PayloadTooLarge,
}

/// Encoding or decoding failure before request dispatch.
#[derive(Debug, Error)]
pub enum ControlCodecError {
    #[error("control payload exceeds the {MAX_CONTROL_PAYLOAD_BYTES}-byte limit")]
    PayloadTooLarge,

    #[error("control payload is not valid Base64: {0}")]
    InvalidBase64(#[source] base64::DecodeError),

    #[error("control payload is not valid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
}

impl ControlCodecError {
    pub fn code(&self) -> ControlErrorCode {
        match self {
            Self::PayloadTooLarge => ControlErrorCode::PayloadTooLarge,
            Self::InvalidBase64(_) | Self::InvalidJson(_) => ControlErrorCode::MalformedRequest,
        }
    }
}

/// Encode a request as one unwrapped Base64 field.
pub fn encode_request(request: &ControlRequestEnvelope) -> Result<String, ControlCodecError> {
    encode_json(request)
}

/// Decode and bound one request field.
pub fn decode_request(payload: &str) -> Result<ControlRequestEnvelope, ControlCodecError> {
    decode_json(payload)
}

/// Encode a response as one unwrapped Base64 field.
pub fn encode_response(response: &ControlResponseEnvelope) -> Result<String, ControlCodecError> {
    encode_json(response)
}

/// Decode a response, primarily for clients and protocol tests.
pub fn decode_response(payload: &str) -> Result<ControlResponseEnvelope, ControlCodecError> {
    decode_json(payload)
}

/// Turn one encoded request into a response without mutating synthesis state.
pub fn process_control_request(payload: &str, server_version: &str) -> ControlResponseEnvelope {
    match decode_request(payload) {
        Ok(request) if request.protocol_version != CONTROL_PROTOCOL_VERSION => error_response(
            Some(request.request_id),
            ControlErrorCode::UnsupportedVersion,
            format!(
                "unsupported control protocol version {}; supported version is {}",
                request.protocol_version, CONTROL_PROTOCOL_VERSION
            ),
        ),
        Ok(request) => match request.request {
            ControlRequest::Capabilities => ControlResponseEnvelope {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                request_id: Some(request.request_id),
                response: ControlResponse::Capabilities {
                    server_version: server_version.to_owned(),
                    supported_protocol_versions: vec![CONTROL_PROTOCOL_VERSION],
                    features: vec![
                        "control_v1".to_owned(),
                        "legacy_commands".to_owned(),
                        "stable_voice_ids".to_owned(),
                    ],
                },
            },
        },
        Err(error) => error_response(None, error.code(), error.to_string()),
    }
}

/// Format a response as one newline-free event record.
pub fn format_control_event(
    response: &ControlResponseEnvelope,
) -> Result<String, ControlCodecError> {
    Ok(format!(
        "{} {}",
        CONTROL_EVENT_PREFIX,
        encode_response(response)?
    ))
}

fn error_response(
    request_id: Option<u64>,
    code: ControlErrorCode,
    message: String,
) -> ControlResponseEnvelope {
    ControlResponseEnvelope {
        protocol_version: CONTROL_PROTOCOL_VERSION,
        request_id,
        response: ControlResponse::Error { code, message },
    }
}

fn encode_json<T: Serialize>(value: &T) -> Result<String, ControlCodecError> {
    let json = serde_json::to_vec(value).map_err(ControlCodecError::InvalidJson)?;
    if json.len() > MAX_CONTROL_PAYLOAD_BYTES {
        return Err(ControlCodecError::PayloadTooLarge);
    }
    Ok(STANDARD.encode(json))
}

fn decode_json<T: DeserializeOwned>(payload: &str) -> Result<T, ControlCodecError> {
    if payload.len() > MAX_CONTROL_ENCODED_BYTES {
        return Err(ControlCodecError::PayloadTooLarge);
    }
    let json = STANDARD
        .decode(payload)
        .map_err(ControlCodecError::InvalidBase64)?;
    if json.len() > MAX_CONTROL_PAYLOAD_BYTES {
        return Err(ControlCodecError::PayloadTooLarge);
    }
    serde_json::from_slice(&json).map_err(ControlCodecError::InvalidJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{
        LogicalVoiceDefinition, NormalizedAcss, PhysicalVoiceId, VoiceSelector,
    };

    fn capabilities_request(version: u32, request_id: u64) -> ControlRequestEnvelope {
        ControlRequestEnvelope {
            protocol_version: version,
            request_id,
            request: ControlRequest::Capabilities,
        }
    }

    #[test]
    fn request_round_trip_is_base64_json() {
        let request = capabilities_request(CONTROL_PROTOCOL_VERSION, 42);
        let encoded = encode_request(&request).unwrap();

        assert!(!encoded.contains('\n'));
        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn capabilities_response_preserves_request_id() {
        let encoded = encode_request(&capabilities_request(1, 73)).unwrap();
        let response = process_control_request(&encoded, "1.3.0");

        assert_eq!(response.request_id, Some(73));
        assert!(matches!(
            response.response,
            ControlResponse::Capabilities { ref server_version, .. }
                if server_version == "1.3.0"
        ));
    }

    #[test]
    fn unsupported_version_returns_structured_error() {
        let encoded = encode_request(&capabilities_request(99, 5)).unwrap();
        let response = process_control_request(&encoded, "1.3.0");

        assert_eq!(response.request_id, Some(5));
        assert!(matches!(
            response.response,
            ControlResponse::Error {
                code: ControlErrorCode::UnsupportedVersion,
                ..
            }
        ));
    }

    #[test]
    fn malformed_payload_returns_unowned_error() {
        let response = process_control_request("not-base64!", "1.3.0");

        assert_eq!(response.request_id, None);
        assert!(matches!(
            response.response,
            ControlResponse::Error {
                code: ControlErrorCode::MalformedRequest,
                ..
            }
        ));
    }

    #[test]
    fn encoded_payload_is_bounded_before_decoding() {
        let payload = "A".repeat(MAX_CONTROL_ENCODED_BYTES + 1);

        assert!(matches!(
            decode_request(&payload),
            Err(ControlCodecError::PayloadTooLarge)
        ));
    }

    #[test]
    fn logical_voice_json_keeps_engine_and_voice_ids_separate() {
        let definition = LogicalVoiceDefinition {
            id: "source-code".to_owned(),
            language: Some("en-US".to_owned()),
            preferences: vec![VoiceSelector::Exact(PhysicalVoiceId::new(
                "winrt",
                r"winrt:HKEY_LOCAL_MACHINE\Voices\David",
            ))],
            acss: NormalizedAcss::default(),
        };

        let json = serde_json::to_value(&definition).unwrap();
        assert_eq!(json["preferences"][0]["kind"], "exact");
        assert_eq!(json["preferences"][0]["engine_id"], "winrt");
        assert_eq!(
            json["preferences"][0]["voice_id"],
            r"winrt:HKEY_LOCAL_MACHINE\Voices\David"
        );
        assert_eq!(
            serde_json::from_value::<LogicalVoiceDefinition>(json).unwrap(),
            definition
        );
    }
}
