use super::*;
use crate::control::{self, ControlRequest, ControlRequestEnvelope, ControlResponse};
use crate::engine_configuration::*;
use crate::engine_registry::EngineRegistry;
use crate::logical_voices::LogicalVoiceRegistry;
use crate::routing_policy::RoutingPolicyRegistry;
use serde_json::json;
use std::collections::BTreeMap;

fn prepared() -> LaunchSnapshot {
    let root = std::env::temp_dir().join("configuration status fixture");
    let mut loaded = LoadedConfiguration {
        root: Some(root.clone()),
        ..LoadedConfiguration::default()
    };
    let manifest = HelperManifest::parse(&serde_json::to_vec(&json!({"schema":1,"engine_id":"org.fixture", "program":root.join("private executable"), "arguments":["private argument"]})).unwrap(), Platform::native()).unwrap();
    loaded.external.insert(
        "org.fixture".into(),
        ManifestRegistration {
            manifest,
            source: root.join("helpers.d/fixture.json"),
        },
    );
    let resolved = ResolvedConfiguration::resolve(
        loaded,
        &root.join("omnivox"),
        Platform::native(),
        LaunchEnvironment::from_variables([("PRIVATE".into(), "private value".into())]),
        &BTreeMap::new(),
    )
    .unwrap();
    LaunchSnapshot::prepare(resolved, None, "org.fixture".into(), false).unwrap()
}

fn response(
    request: ControlRequest,
    status: Option<&EngineConfigurationStatus>,
) -> ControlResponse {
    let payload = control::encode_request(&ControlRequestEnvelope {
        protocol_version: 1,
        request_id: 71,
        request,
    })
    .unwrap();
    let response = control::process_control_request_with_configuration(
        &payload,
        "test",
        9,
        "org.fixture",
        &[],
        &[],
        &mut LogicalVoiceRegistry::default(),
        &mut RoutingPolicyRegistry::new("org.fixture"),
        None,
        &[],
        status,
    );
    assert_eq!(response.request_id, Some(71));
    response.response
}

#[test]
fn acknowledgement_comes_from_bound_snapshot_and_cannot_be_rebound() {
    let snapshot = prepared();
    let mut registry = EngineRegistry::new();
    assert!(registry.engine_configuration_status(0, &[]).is_none());
    registry.configure_startup_snapshot(&snapshot).unwrap();
    assert!(registry.configure_startup_snapshot(&prepared()).is_err());
    assert!(registry
        .configure_local_selection(
            LocalRoutingPolicy::default(),
            EngineSelectionPermissions::default()
        )
        .is_err());
    registry
        .register_unavailable(
            EngineDescriptor::unavailable("org.fixture", "runtime is absent"),
            || Err("absent".into()),
        )
        .unwrap();
    let status = registry
        .engine_configuration_status(9, &registry.inventory())
        .unwrap();
    assert_eq!(status.activation_id, snapshot.activation_id());
    assert_eq!(status.inventory_generation, 9);
    let external = status
        .registrations
        .iter()
        .find(|entry| entry.engine_id == "org.fixture")
        .unwrap();
    assert_eq!(external.origin, EngineOrigin::ExternalHelper);
    assert_eq!(
        external.source.as_deref(),
        Some(
            std::path::Path::new("helpers.d/fixture.json")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(
        external.availability,
        Availability::Unavailable {
            reason: "runtime is absent".into()
        }
    );
    let encoded = serde_json::to_string(&status).unwrap();
    for private in [
        "private executable",
        "private argument",
        "private value",
        "PRIVATE",
    ] {
        assert!(!encoded.contains(private));
    }
    assert!(
        matches!(response(ControlRequest::EngineConfigurationStatusV1, Some(&status)), ControlResponse::EngineConfigurationStatusV1(actual) if actual == status)
    );
    assert!(matches!(
        response(ControlRequest::EngineConfigurationStatusV1, None),
        ControlResponse::Error {
            code: control::ControlErrorCode::UnsupportedOperation,
            ..
        }
    ));
    for bound in [false, true] {
        let ControlResponse::Capabilities { features, .. } =
            response(ControlRequest::Capabilities, bound.then_some(&status))
        else {
            panic!("expected capabilities")
        };
        assert_eq!(
            features
                .iter()
                .any(|feature| feature == "engine_configuration_v1"),
            bound
        );
    }
}

#[test]
fn status_is_read_only_and_rejects_executable_definitions_or_duplicate_keys() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    for extra in [
        r#", "program":"/must/not/run""#,
        r#", "activation_id":"forged""#,
        r#", "request_id":72"#,
        r#", "ty\u0070e":"engine_configuration_status_v1""#,
    ] {
        let bytes = format!(
            r#"{{"protocol_version":1,"request_id":71,"type":"engine_configuration_status_v1"{extra}}}"#
        );
        assert!(control::decode_request(&STANDARD.encode(bytes)).is_err());
    }
}

#[test]
fn unavailable_reason_is_utf8_bounded_without_losing_the_identity() {
    let mut status = EngineConfigurationStatus::from_snapshot(&prepared());
    let engine = EngineDescriptor::unavailable("org.fixture", "🗣".repeat(10000));
    status = status.with_inventory(4, &[engine]);
    let external = status
        .registrations
        .iter()
        .find(|entry| entry.engine_id == "org.fixture")
        .unwrap();
    let Availability::Unavailable { reason } = &external.availability else {
        panic!("expected unavailable")
    };
    assert!(reason.len() <= 512 && reason.ends_with('…'));
    let response = control::ControlResponseEnvelope {
        protocol_version: 1,
        request_id: Some(71),
        response: ControlResponse::EngineConfigurationStatusV1(status),
    };
    let encoded = control::encode_response(&response).unwrap();
    assert_eq!(control::decode_response(&encoded).unwrap(), response);
}

#[test]
fn launch_failure_does_not_disclose_private_launch_fields_in_status_reasons() {
    let path = std::env::temp_dir().join(format!(
        "missing-private-helper-{}",
        crate::voice_library::local::new_uuid().unwrap()
    ));
    let mut config = crate::helper_engine::HelperEngineConfig::with_environment(
        "org.fixture",
        path.clone(),
        LaunchEnvironment::from_variables([("PRIVATE".into(), "private environment".into())]),
    );
    config.arguments = vec!["private argument".into()];
    let engine = crate::helper_engine::HelperTtsEngine::prepare(config).unwrap();
    let error = std::sync::Arc::new(engine)
        .initialize_before(std::time::Instant::now() + std::time::Duration::from_secs(1))
        .unwrap_err()
        .to_string();
    assert!(error.contains("org.fixture"));
    assert!(!error.contains(&path.to_string_lossy().to_string()));
    assert!(!error.contains("private argument") && !error.contains("private environment"));
}
