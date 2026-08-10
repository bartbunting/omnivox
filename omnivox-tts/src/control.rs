//! Versioned Base64-JSON control protocol.
//!
//! Presentation transactions remain in the legacy line protocol. This channel
//! carries discovery, configuration, and diagnostic messages without making
//! native identifiers part of Tcl command syntax.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::{
    AcssDimension, EngineDescriptor, FallbackPolicy, LogicalVoiceDefinition, NormalizedAcss,
    PhysicalVoiceId, PostSynthesisDimension, PostSynthesisStyle, VoiceSelector,
};
use crate::logical_voices::{
    LogicalVoiceRegistration, LogicalVoiceRegistry, LogicalVoiceRegistryError,
};
use crate::routing_policy::{
    RoutingPolicy, RoutingPolicyError, RoutingPolicyRegistration, RoutingPolicyRegistry,
};

/// Current control protocol version.
pub const CONTROL_PROTOCOL_VERSION: u32 = 1;

/// Maximum decoded JSON payload accepted from a client.
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 256 * 1024;

/// Conservative maximum Base64 size for the decoded payload bound.
pub const MAX_CONTROL_ENCODED_BYTES: usize = (MAX_CONTROL_PAYLOAD_BYTES / 3) * 4 + 8;

/// Maximum UTF-8 text accepted by one transactional preview request.
pub const MAX_PREVIEW_TEXT_BYTES: usize = 16 * 1024;

/// Legacy commands reported as parsed but unsupported by current policy.
///
/// The CLI has a cross-layer test against the parser crate's corresponding
/// list so capability negotiation cannot drift from command admission.
pub const DEPRECATED_PROTOCOL_COMMANDS: &[&str] = &[
    "set_lang",
    "set_next_lang",
    "set_previous_lang",
    "set_preferred_lang",
    "tts_set_notification_channel",
];

/// Prefix used for machine-readable server events on stdout.
pub const CONTROL_EVENT_PREFIX: &str = "__OMNIVOX_CONTROL__";

/// One versioned client request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRequestEnvelope {
    pub protocol_version: u32,
    pub request_id: u64,
    #[serde(flatten)]
    pub request: ControlRequest,
}

/// Requests implemented by the control channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlRequest {
    Capabilities,
    Inventory,
    RegisterLogicalVoices {
        registry_generation: u64,
        definitions: Vec<LogicalVoiceDefinition>,
        #[serde(default)]
        fallback_policy: FallbackPolicy,
    },
    SetRoutingPolicy {
        routing_policy_generation: u64,
        #[serde(flatten)]
        policy: RoutingPolicy,
    },
    RequestEngineRecoveryProbe {
        engine_id: String,
    },
    Preview {
        text: String,
        selector: VoiceSelector,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        acss: NormalizedAcss,
        /// Signed points relative to the server's current normalized speech
        /// rate. Mutually exclusive with `acss.rate`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate_offset: Option<i16>,
        #[serde(default)]
        effects: PostSynthesisStyle,
    },
}

/// One versioned server response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponseEnvelope {
    pub protocol_version: u32,
    pub request_id: Option<u64>,
    #[serde(flatten)]
    pub response: ControlResponse,
}

/// Response payloads emitted by the control channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlResponse {
    Capabilities {
        server_version: String,
        supported_protocol_versions: Vec<u32>,
        features: Vec<String>,
        /// Parsed commands that return an explicit unsupported response.
        #[serde(default)]
        deprecated_commands: Vec<String>,
    },
    Inventory {
        inventory_generation: u64,
        preferred_engine_id: String,
        routing_policy: RoutingPolicyRegistration,
        engine_runtime: Vec<EngineRuntimeStatus>,
        engines: Vec<EngineDescriptor>,
    },
    LogicalVoicesRegistered {
        inventory_generation: u64,
        registration: LogicalVoiceRegistration,
    },
    RoutingPolicyApplied {
        inventory_generation: u64,
        routing_policy: RoutingPolicyRegistration,
        logical_voices: LogicalVoiceRegistration,
    },
    EngineRecoveryProbeRequested {
        inventory_generation: u64,
        engine_id: String,
    },
    PreviewCompleted {
        status: PreviewStatus,
        requested: VoiceSelector,
        realized: Option<PhysicalVoiceId>,
        degraded_acss: Vec<AcssDimension>,
        degraded_effects: Vec<PostSynthesisDimension>,
        message: Option<String>,
    },
    Error {
        code: ControlErrorCode,
        message: String,
    },
}

/// Terminal state of a one-shot voice preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    Completed,
    Cancelled,
    Failed,
}

/// Runtime circuit state exposed separately from static engine capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCircuitStatus {
    Closed,
    Cooldown,
    Ready,
    Probing,
}

/// Dynamic operational state for one inventory engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineRuntimeStatus {
    pub engine_id: String,
    pub circuit: EngineCircuitStatus,
    pub last_failure: Option<String>,
    pub cooldown_remaining_ms: Option<u64>,
    pub disabled_by_policy: bool,
}

/// Stable machine-readable control error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode {
    MalformedRequest,
    UnsupportedVersion,
    UnsupportedOperation,
    PayloadTooLarge,
    InvalidConfiguration,
    StaleGeneration,
    GenerationConflict,
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
// Keep the protocol projections explicit at this boundary; bundling them would hide
// which server snapshots are used to construct a response.
#[allow(clippy::too_many_arguments)]
pub fn process_control_request(
    payload: &str,
    server_version: &str,
    inventory_generation: u64,
    preferred_engine_id: &str,
    engines: &[EngineDescriptor],
    engine_runtime: &[EngineRuntimeStatus],
    logical_voices: &mut LogicalVoiceRegistry,
    routing_policy: &mut RoutingPolicyRegistry,
) -> ControlResponseEnvelope {
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
                        "capitalization_presentation_v1".to_owned(),
                        "control_v1".to_owned(),
                        "emacsvox_tx".to_owned(),
                        "engine_inventory".to_owned(),
                        "engine_recovery_probe".to_owned(),
                        "exact_voice_preview".to_owned(),
                        "legacy_commands".to_owned(),
                        "logical_voice_registration".to_owned(),
                        "logical_voice_language_routing".to_owned(),
                        "logical_voice_routing".to_owned(),
                        "playback_marker_events_v1".to_owned(),
                        "playback_marker_events_v2".to_owned(),
                        "presentation_timeline_v1".to_owned(),
                        "presentation_timeline_v2".to_owned(),
                        "presentation_timeline_v3".to_owned(),
                        "presentation_tone_v1".to_owned(),
                        "post_synthesis_effects_v1".to_owned(),
                        "preferred_engine".to_owned(),
                        "process_audio_routing".to_owned(),
                        "relative_rate_v1".to_owned(),
                        "runtime_routing_policy".to_owned(),
                        "stable_voice_ids".to_owned(),
                        "text_repertoire_routing_v1".to_owned(),
                        "tracked_playback_completion".to_owned(),
                    ],
                    deprecated_commands: DEPRECATED_PROTOCOL_COMMANDS
                        .iter()
                        .map(|command| (*command).to_owned())
                        .collect(),
                },
            },
            ControlRequest::Inventory => {
                let projected = routing_policy.project_inventory(engines.to_vec());
                ControlResponseEnvelope {
                    protocol_version: CONTROL_PROTOCOL_VERSION,
                    request_id: Some(request.request_id),
                    response: ControlResponse::Inventory {
                        inventory_generation: routing_policy
                            .inventory_generation(inventory_generation),
                        preferred_engine_id: routing_policy
                            .policy()
                            .preferred_engine_ids
                            .first()
                            .cloned()
                            .unwrap_or_else(|| preferred_engine_id.to_owned()),
                        routing_policy: routing_policy.registration(),
                        engine_runtime: engine_runtime.to_vec(),
                        engines: projected,
                    },
                }
            }
            ControlRequest::RegisterLogicalVoices {
                registry_generation,
                definitions,
                fallback_policy,
            } => match logical_voices.register(
                registry_generation,
                definitions,
                fallback_policy,
                &routing_policy.project_inventory(engines.to_vec()),
            ) {
                Ok(_) => {
                    let projected = routing_policy.project_inventory(engines.to_vec());
                    let effective = routing_policy
                        .effective_fallback_policy(logical_voices.fallback_policy());
                    let registration = logical_voices
                        .resolve_and_store_with_policy(&projected, &effective);
                    ControlResponseEnvelope {
                        protocol_version: CONTROL_PROTOCOL_VERSION,
                        request_id: Some(request.request_id),
                        response: ControlResponse::LogicalVoicesRegistered {
                            inventory_generation: routing_policy
                                .inventory_generation(inventory_generation),
                            registration,
                        },
                    }
                }
                Err(error) => error_response(
                    Some(request.request_id),
                    registry_error_code(&error),
                    error.to_string(),
                ),
            },
            ControlRequest::SetRoutingPolicy {
                routing_policy_generation,
                policy,
            } => match routing_policy.register(routing_policy_generation, policy) {
                Ok(registration) => {
                    let projected = routing_policy.project_inventory(engines.to_vec());
                    let effective = routing_policy
                        .effective_fallback_policy(logical_voices.fallback_policy());
                    let logical_registration = logical_voices
                        .resolve_and_store_with_policy(&projected, &effective);
                    ControlResponseEnvelope {
                        protocol_version: CONTROL_PROTOCOL_VERSION,
                        request_id: Some(request.request_id),
                        response: ControlResponse::RoutingPolicyApplied {
                            inventory_generation: routing_policy
                                .inventory_generation(inventory_generation),
                            routing_policy: registration,
                            logical_voices: logical_registration,
                        },
                    }
                }
                Err(error) => error_response(
                    Some(request.request_id),
                    policy_error_code(&error),
                    error.to_string(),
                ),
            },
            ControlRequest::Preview { .. }
            | ControlRequest::RequestEngineRecoveryProbe { .. } => error_response(
                Some(request.request_id),
                ControlErrorCode::InvalidConfiguration,
                "request requires a live playback server".to_owned(),
            ),
        },
        Err(error) => error_response(None, error.code(), error.to_string()),
    }
}

fn policy_error_code(error: &RoutingPolicyError) -> ControlErrorCode {
    match error {
        RoutingPolicyError::StaleGeneration { .. } => ControlErrorCode::StaleGeneration,
        RoutingPolicyError::GenerationConflict { .. } => ControlErrorCode::GenerationConflict,
        _ => ControlErrorCode::InvalidConfiguration,
    }
}

fn registry_error_code(error: &LogicalVoiceRegistryError) -> ControlErrorCode {
    match error {
        LogicalVoiceRegistryError::StaleGeneration { .. } => ControlErrorCode::StaleGeneration,
        LogicalVoiceRegistryError::GenerationConflict { .. } => {
            ControlErrorCode::GenerationConflict
        }
        _ => ControlErrorCode::InvalidConfiguration,
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
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, EngineHealth, LogicalVoiceDefinition, MarkerCapabilities,
        NormalizedAcss, PhysicalVoiceId, VoiceDescriptor, VoiceSelector,
    };
    use crate::logical_voices::LogicalVoiceBinding;
    use crate::VoiceQuality;

    fn capabilities_request(version: u32, request_id: u64) -> ControlRequestEnvelope {
        ControlRequestEnvelope {
            protocol_version: version,
            request_id,
            request: ControlRequest::Capabilities,
        }
    }

    fn process_without_registry(
        payload: &str,
        server_version: &str,
        inventory_generation: u64,
        engines: &[EngineDescriptor],
    ) -> ControlResponseEnvelope {
        process_control_request(
            payload,
            server_version,
            inventory_generation,
            engines.first().map_or("", |engine| engine.id.as_str()),
            engines,
            &[],
            &mut LogicalVoiceRegistry::default(),
            &mut RoutingPolicyRegistry::new(
                engines.first().map_or("", |engine| engine.id.as_str()),
            ),
        )
    }

    fn inventory() -> Vec<EngineDescriptor> {
        vec![EngineDescriptor {
            id: "winrt".to_owned(),
            display_name: "Windows WinRT Speech Synthesis".to_owned(),
            version: None,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities::default(),
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::PlaybackOnly,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: true,
                text_repertoire: crate::contracts::TextRepertoire::Unicode,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices: vec![VoiceDescriptor {
                id: PhysicalVoiceId::new("winrt", "winrt:David"),
                display_name: "David".to_owned(),
                language: Some("en-US".to_owned()),
                gender: None,
                quality: VoiceQuality::Enhanced,
                availability: Availability::Available,
            }],
            default_voice_id: Some("winrt:David".to_owned()),
        }]
    }

    fn logical_voice(id: &str) -> LogicalVoiceDefinition {
        LogicalVoiceDefinition {
            id: id.to_owned(),
            language: Some("en-US".to_owned()),
            preferences: vec![VoiceSelector::Exact(PhysicalVoiceId::new(
                "winrt",
                "winrt:David",
            ))],
            acss: NormalizedAcss::default(),
            effects: Default::default(),
        }
    }

    fn registration_request(
        request_id: u64,
        registry_generation: u64,
        definitions: Vec<LogicalVoiceDefinition>,
    ) -> ControlRequestEnvelope {
        ControlRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id,
            request: ControlRequest::RegisterLogicalVoices {
                registry_generation,
                definitions,
                fallback_policy: FallbackPolicy::default(),
            },
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
        let response = process_without_registry(&encoded, "1.3.0", 1, &[]);

        assert_eq!(response.request_id, Some(73));
        assert!(matches!(
            response.response,
            ControlResponse::Capabilities {
                ref server_version,
                ref features,
                ref deprecated_commands,
                ..
            } if server_version == "1.3.0"
                && features
                    .iter()
                    .any(|feature| feature == "capitalization_presentation_v1")
                && features.iter().any(|feature| feature == "emacsvox_tx")
                && features
                    .iter()
                    .any(|feature| feature == "exact_voice_preview")
                && features.iter().any(|feature| feature == "logical_voice_routing")
                && features
                    .iter()
                    .any(|feature| feature == "logical_voice_language_routing")
                && features
                    .iter()
                    .any(|feature| feature == "presentation_timeline_v2")
                && features
                    .iter()
                    .any(|feature| feature == "presentation_timeline_v3")
                && features
                    .iter()
                    .any(|feature| feature == "presentation_tone_v1")
                && features.iter().any(|feature| feature == "relative_rate_v1")
                && features.iter().any(|feature| feature == "runtime_routing_policy")
                && features.iter().any(|feature| feature == "process_audio_routing")
                && features.iter().any(|feature| feature == "engine_recovery_probe")
                && features
                    .iter()
                    .any(|feature| feature == "playback_marker_events_v1")
                && features
                    .iter()
                    .any(|feature| feature == "tracked_playback_completion")
                && features
                    .iter()
                    .any(|feature| feature == "text_repertoire_routing_v1")
                && deprecated_commands
                    == &DEPRECATED_PROTOCOL_COMMANDS
                        .iter()
                        .map(|command| (*command).to_owned())
                        .collect::<Vec<_>>()
        ));
    }

    #[test]
    fn unsupported_version_returns_structured_error() {
        let encoded = encode_request(&capabilities_request(99, 5)).unwrap();
        let response = process_without_registry(&encoded, "1.3.0", 1, &[]);

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
        let response = process_without_registry("not-base64!", "1.3.0", 1, &[]);

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
            effects: Default::default(),
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

    #[test]
    fn preview_request_round_trip_preserves_exact_route_and_unsaved_acss() {
        let request = ControlRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: 88,
            request: ControlRequest::Preview {
                text: "Compare this voice.".to_owned(),
                selector: VoiceSelector::Exact(PhysicalVoiceId::new("winrt", "winrt:David")),
                language: Some("en-US".to_owned()),
                acss: NormalizedAcss {
                    richness: Some(0.4),
                    ..NormalizedAcss::default()
                },
                rate_offset: Some(-4),
                effects: PostSynthesisStyle {
                    reverb: Some(0.4),
                    ..PostSynthesisStyle::default()
                },
            },
        };

        let encoded = encode_request(&request).unwrap();

        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn inventory_response_carries_generation_and_descriptors() {
        let engine = inventory().remove(0);
        let request = ControlRequestEnvelope {
            protocol_version: 1,
            request_id: 91,
            request: ControlRequest::Inventory,
        };
        let encoded = encode_request(&request).unwrap();

        let response =
            process_without_registry(&encoded, "1.3.0", 7, std::slice::from_ref(&engine));

        assert!(matches!(
            response.response,
            ControlResponse::Inventory {
                inventory_generation: 7,
                preferred_engine_id: ref preferred,
                ref engines,
                ..
            } if preferred == "winrt" && engines == &[engine]
        ));
    }

    #[test]
    fn logical_voice_registration_commits_and_reports_resolution() {
        let encoded = encode_request(&registration_request(
            101,
            3,
            vec![logical_voice("source-code")],
        ))
        .unwrap();
        let mut registry = LogicalVoiceRegistry::default();

        let response = process_control_request(
            &encoded,
            "1.3.0",
            7,
            "winrt",
            &inventory(),
            &[],
            &mut registry,
            &mut RoutingPolicyRegistry::new("winrt"),
        );

        assert_eq!(response.request_id, Some(101));
        assert_eq!(registry.generation(), 3);
        assert!(matches!(
            response.response,
            ControlResponse::LogicalVoicesRegistered {
                inventory_generation: 7,
                registration: LogicalVoiceRegistration {
                    registry_generation: 3,
                    ref bindings,
                },
            } if matches!(bindings.as_slice(), [LogicalVoiceBinding::Resolved { .. }])
        ));
    }

    #[test]
    fn routing_policy_applies_atomically_and_re_resolves_logical_voices() {
        let request = ControlRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: 92,
            request: ControlRequest::SetRoutingPolicy {
                routing_policy_generation: 4,
                policy: RoutingPolicy {
                    preferred_engine_ids: vec!["eloquence".to_owned(), "winrt".to_owned()],
                    fallback_engine_ids: vec!["espeak".to_owned()],
                    disabled_engine_ids: vec!["winrt".to_owned()],
                },
            },
        };
        let encoded = encode_request(&request).unwrap();
        let mut logical = LogicalVoiceRegistry::default();
        let mut policy = RoutingPolicyRegistry::new("winrt");

        let response = process_control_request(
            &encoded,
            "1.3.0",
            7,
            "winrt",
            &inventory(),
            &[],
            &mut logical,
            &mut policy,
        );

        assert_eq!(policy.generation(), 4);
        assert_eq!(policy.policy().disabled_engine_ids, ["winrt"]);
        assert!(matches!(
            response.response,
            ControlResponse::RoutingPolicyApplied {
                inventory_generation: 11,
                routing_policy: RoutingPolicyRegistration {
                    routing_policy_generation: 4,
                    ..
                },
                logical_voices: LogicalVoiceRegistration {
                    registry_generation: 0,
                    ..
                },
            }
        ));
    }

    #[test]
    fn inventory_projects_policy_disablement_without_losing_order() {
        let encoded = encode_request(&ControlRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: 93,
            request: ControlRequest::Inventory,
        })
        .unwrap();
        let mut policy = RoutingPolicyRegistry::new("winrt");
        policy
            .register(
                2,
                RoutingPolicy {
                    preferred_engine_ids: vec!["winrt".to_owned()],
                    fallback_engine_ids: vec!["espeak".to_owned()],
                    disabled_engine_ids: vec!["winrt".to_owned()],
                },
            )
            .unwrap();
        let response = process_control_request(
            &encoded,
            "1.3.0",
            7,
            "winrt",
            &inventory(),
            &[],
            &mut LogicalVoiceRegistry::default(),
            &mut policy,
        );

        assert!(matches!(
            response.response,
            ControlResponse::Inventory {
                inventory_generation: 9,
                routing_policy: RoutingPolicyRegistration {
                    policy: RoutingPolicy { ref preferred_engine_ids, ref disabled_engine_ids, .. },
                    ..
                },
                ref engines,
                ..
            } if preferred_engine_ids == &["winrt"]
                && disabled_engine_ids == &["winrt"]
                && matches!(engines[0].availability, Availability::Unavailable { .. })
        ));
    }

    #[test]
    fn stale_registration_returns_a_request_owned_error() {
        let mut registry = LogicalVoiceRegistry::default();
        registry
            .register(
                4,
                vec![logical_voice("source-code")],
                FallbackPolicy::default(),
                &inventory(),
            )
            .unwrap();
        let encoded = encode_request(&registration_request(
            102,
            3,
            vec![logical_voice("annotation")],
        ))
        .unwrap();

        let response = process_control_request(
            &encoded,
            "1.3.0",
            7,
            "winrt",
            &inventory(),
            &[],
            &mut registry,
            &mut RoutingPolicyRegistry::new("winrt"),
        );

        assert_eq!(response.request_id, Some(102));
        assert!(matches!(
            response.response,
            ControlResponse::Error {
                code: ControlErrorCode::StaleGeneration,
                ..
            }
        ));
        assert_eq!(registry.generation(), 4);
    }

    #[test]
    fn invalid_registration_returns_a_configuration_error() {
        let encoded = encode_request(&registration_request(
            103,
            1,
            vec![logical_voice("invalid voice")],
        ))
        .unwrap();
        let mut registry = LogicalVoiceRegistry::default();

        let response = process_control_request(
            &encoded,
            "1.3.0",
            7,
            "winrt",
            &inventory(),
            &[],
            &mut registry,
            &mut RoutingPolicyRegistry::new("winrt"),
        );

        assert!(matches!(
            response.response,
            ControlResponse::Error {
                code: ControlErrorCode::InvalidConfiguration,
                ..
            }
        ));
        assert_eq!(registry.generation(), 0);
        assert!(registry.definitions().is_empty());
    }
}
