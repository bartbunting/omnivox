use super::*;
use crate::contracts::{Availability, EngineHealth, VoiceDescriptor};
use crate::engine_voice_choices::NativeSupport;
use crate::native_parameters::ParameterCatalogue;
use crate::{VoiceInfo, VoiceQuality};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let value: Value = serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    value["messages"][name].clone()
}
fn inventory() -> Vec<EngineDescriptor> {
    [("dectalk", "paul"), ("eloquence", "v1"), ("espeak", "en")]
        .into_iter()
        .map(|(engine, voice)| {
            let mut d = EngineDescriptor::unavailable(engine, "fixture");
            d.availability = Availability::Available;
            d.health = EngineHealth::Healthy;
            d.voices.push(VoiceDescriptor::from_voice_info(
                engine,
                VoiceInfo {
                    identifier: voice.into(),
                    name: voice.into(),
                    language: "en-US".into(),
                    quality: VoiceQuality::Compact,
                },
            ));
            d.default_voice_id = Some(voice.into());
            d
        })
        .collect()
}
fn catalogues() -> Vec<ParameterCatalogue> {
    let mut value = fixture("catalogue_response")["result"].clone();
    let object = value.as_object_mut().unwrap();
    object.remove("status");
    object.remove("next_cursor");
    object.insert("engine_id".into(), json!("dectalk"));
    let dectalk = ParameterCatalogue::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    let mut eci = dectalk.clone();
    eci.engine_id = "eloquence".into();
    eci.voice_id = None;
    eci.identity.schema_id = "eloquence.eci-units.v1".into();
    eci.parameters.truncate(1);
    eci.parameters[0].id = "breathiness".into();
    eci.parameters[0].default.source = crate::native_parameters::DefaultSource::Unknown;
    eci.parameters[0].default.value = None;
    eci.mappings.clear();
    vec![dectalk, eci]
}
fn process(
    raw: &str,
    registry: &mut LogicalVoiceRegistry,
    policy: &mut RoutingPolicyRegistry,
    inventory: &[EngineDescriptor],
    catalogues: &[ParameterCatalogue],
) -> ControlResponseEnvelope {
    let knowledge = catalogues
        .iter()
        .map(ParameterKnowledge::Ready)
        .collect::<Vec<_>>();
    process_control_request_with_parameters(
        &STANDARD.encode(raw),
        "test",
        12,
        "",
        inventory,
        &[],
        registry,
        policy,
        None,
        &knowledge,
    )
}
fn statuses(
    response: &ControlResponseEnvelope,
) -> &[crate::engine_voice_choices::NativeChoiceStatus] {
    let ControlResponse::LogicalVoicesRegisteredV3 { native_status, .. } = &response.response
    else {
        panic!("{response:?}")
    };
    native_status
}
fn error(response: &ControlResponseEnvelope, code: ControlErrorCode) {
    assert!(
        matches!(response.response, ControlResponse::Error { code: actual, .. } if actual == code),
        "{response:?}"
    );
}

#[test]
fn native_registration_fixture_roundtrips_and_advertises_complete_execution() {
    let raw = fixture("registration").to_string();
    let request = decode_request(&STANDARD.encode(&raw)).unwrap();
    assert_eq!(
        decode_request(&encode_request(&request).unwrap()).unwrap(),
        request
    );
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    let response = process(
        &raw,
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        fixture("registration_ack")
    );
    assert_eq!(
        decode_response(&encode_response(&response).unwrap()).unwrap(),
        response
    );
    assert_eq!(
        process(
            &raw,
            &mut registry,
            &mut policy,
            &inventory(),
            &catalogues()
        ),
        response
    );
    assert!(registry.is_engine_layered("bolden"));
    let caps = process(
        r#"{"protocol_version":1,"request_id":9,"type":"capabilities"}"#,
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    let ControlResponse::Capabilities { features, .. } = caps.response else {
        panic!()
    };
    for reserved in [
        "engine_voice_parameters_v1",
        "presentation_timeline_v5",
        "playback_marker_events_v4",
    ] {
        assert!(features.iter().any(|feature| feature == reserved));
    }
}

#[test]
fn metadata_can_change_status_on_idempotent_retry_without_rewriting_saved_settings() {
    let raw = fixture("registration").to_string();
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    let missing = process(&raw, &mut registry, &mut policy, &[], &[]);
    assert!(statuses(&missing)
        .iter()
        .all(|s| s.status == NativeSupport::Unavailable && s.reason.is_some()));
    let saved = registry.registered_definitions().to_vec();
    let deferred = process(&raw, &mut registry, &mut policy, &inventory(), &[]);
    assert!(statuses(&deferred)
        .iter()
        .all(|s| s.status == NativeSupport::Deferred));
    let supported = process(
        &raw,
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    assert!(statuses(&supported)
        .iter()
        .all(|s| s.status == NativeSupport::Supported && s.reason.is_none()));
    let mut unknown = catalogues();
    unknown[0].identity.schema_id = "other.schema".into();
    let unavailable = process(&raw, &mut registry, &mut policy, &inventory(), &unknown);
    assert_eq!(statuses(&unavailable)[0].status, NativeSupport::Unavailable);
    assert_eq!(registry.registered_definitions(), saved);
    assert_eq!(registry.generation(), 41);
}

#[test]
fn invalid_values_generations_ids_and_conflicting_metadata_never_publish() {
    let original = fixture("registration");
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    process(
        &original.to_string(),
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    let saved = registry.registered_definitions().to_vec();
    for (pointer, value, code) in [
        (
            "/registry_generation",
            json!(40),
            ControlErrorCode::StaleGeneration,
        ),
        (
            "/registry_generation",
            json!(0),
            ControlErrorCode::InvalidConfiguration,
        ),
        (
            "/request_id",
            json!(0),
            ControlErrorCode::InvalidConfiguration,
        ),
        (
            "/protocol_version",
            json!(2),
            ControlErrorCode::UnsupportedVersion,
        ),
        (
            "/definitions/0/definition/choices/0/id",
            json!("changed"),
            ControlErrorCode::GenerationConflict,
        ),
        (
            "/definitions/0/definition/choices/0/native/parameters/sm/value",
            json!(101),
            ControlErrorCode::InvalidConfiguration,
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        error(
            &process(
                &changed.to_string(),
                &mut registry,
                &mut policy,
                &inventory(),
                &catalogues(),
            ),
            code,
        );
        assert_eq!(registry.generation(), 41);
        assert_eq!(registry.registered_definitions(), saved);
    }
    let mut changed = original;
    changed["registry_generation"] = json!(42);
    changed["definitions"][0]["definition"]["choices"][0]["native"]["parameters"]["sm"]["value"] =
        json!(101);
    error(
        &process(
            &changed.to_string(),
            &mut registry,
            &mut policy,
            &inventory(),
            &catalogues(),
        ),
        ControlErrorCode::InvalidConfiguration,
    );
    changed["definitions"][0]["definition"]["choices"][0]["native"]["parameters"]["sm"]["value"] =
        json!(0);
    let mut conflicting = catalogues();
    conflicting.push(conflicting[0].clone());
    error(
        &process(
            &changed.to_string(),
            &mut registry,
            &mut policy,
            &inventory(),
            &conflicting,
        ),
        ControlErrorCode::InvalidConfiguration,
    );
    assert_eq!(registry.generation(), 41);
    assert_eq!(registry.registered_definitions(), saved);
}

#[test]
fn malformed_native_envelopes_preserve_correlation_and_registry() {
    let original = fixture("registration");
    let mut cases = Vec::new();
    for path in [
        "",
        "/fallback_policy",
        "/definitions/0",
        "/definitions/0/definition",
        "/definitions/0/definition/choices/0/native",
        "/definitions/0/definition/choices/0/selector",
    ] {
        let mut changed = original.clone();
        changed.pointer_mut(path).unwrap()["unknown"] = json!(true);
        cases.push((changed.to_string(), Some(801)));
    }
    for path in [
        "/fallback_policy/global_default",
        "/definitions/0/definition/choices/0/native",
    ] {
        let mut changed = original.clone();
        let (parent, key) = path.rsplit_once('/').unwrap();
        changed
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        cases.push((changed.to_string(), Some(801)));
    }
    let raw = original.to_string();
    for (needle, replacement, request_id) in [
        (
            "\"request_id\":801",
            "\"request_id\":801,\"request_id\":802",
            None,
        ),
        (
            "\"type\":\"register_logical_voices_v3\"",
            "\"type\":\"register_logical_voices_v3\",\"type\":\"inventory\"",
            Some(801),
        ),
        ("\"sm\":{", "\"sm\":{},\"sm\":{", Some(801)),
        (
            "\"op\":\"set\"",
            "\"op\":\"set\",\"op\":\"default\"",
            Some(801),
        ),
    ] {
        assert!(raw.contains(needle));
        cases.push((raw.replacen(needle, replacement, 1), request_id));
    }
    for (raw, id) in cases {
        let mut registry = LogicalVoiceRegistry::default();
        let result = process(
            &raw,
            &mut registry,
            &mut RoutingPolicyRegistry::new(""),
            &inventory(),
            &catalogues(),
        );
        error(&result, ControlErrorCode::MalformedRequest);
        assert_eq!(result.request_id, id);
        assert_eq!(registry.generation(), 0);
    }
}

#[test]
fn policy_disablement_and_effective_fallback_control_public_acknowledgement() {
    let mut request = fixture("registration");
    // Native settings remain saved when their requested engine cannot run.
    request["definitions"][0]["definition"]["choices"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    policy
        .register(
            3,
            RoutingPolicy {
                disabled_engine_ids: vec!["dectalk".into()],
                fallback_engine_ids: vec![],
                preferred_engine_ids: vec![],
            },
        )
        .unwrap();
    let response = process(
        &request.to_string(),
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    assert_eq!(statuses(&response)[0].status, NativeSupport::Unavailable);
    let ControlResponse::LogicalVoicesRegisteredV3 {
        unresolved_logical_voice_ids,
        inventory_generation,
        ..
    } = response.response
    else {
        panic!()
    };
    assert_eq!(unresolved_logical_voice_ids, vec!["bolden"]);
    assert_eq!(inventory_generation, policy.inventory_generation(12));
    // Applying a workstation fallback resolves ordinary common speech, but does
    // not turn disabled native settings into supported settings.
    policy
        .register(
            4,
            RoutingPolicy {
                disabled_engine_ids: vec!["dectalk".into()],
                fallback_engine_ids: vec!["espeak".into()],
                preferred_engine_ids: vec![],
            },
        )
        .unwrap();
    let response = process(
        &request.to_string(),
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    assert_eq!(statuses(&response)[0].status, NativeSupport::Unavailable);
    let ControlResponse::LogicalVoicesRegisteredV3 {
        unresolved_logical_voice_ids,
        ..
    } = response.response
    else {
        panic!()
    };
    assert!(unresolved_logical_voice_ids.is_empty());
}

#[test]
fn mixed_definitions_share_generations_and_cannot_smuggle_native_fields_into_old_modes() {
    let mut request = fixture("registration");
    let common: Value = serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/voice-choice-tuning.json"
    ))
    .unwrap();
    let mut layered = common["messages"]["registration"]["definitions"][0].clone();
    layered["definition"]["id"] = json!("layered");
    request["definitions"].as_array_mut().unwrap().extend([
        layered,
        json!({"mode":"legacy","definition":{"id":"legacy","preferences":[],"acss":{}}}),
    ]);
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    let result = process(
        &request.to_string(),
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    assert_eq!(statuses(&result).len(), 3);
    assert_eq!(registry.registered_definitions().len(), 3);
    for index in [1, 2] {
        let mut changed = request.clone();
        changed["registry_generation"] = json!(42);
        changed["definitions"][index]["definition"]["native"] = json!({});
        error(
            &process(
                &changed.to_string(),
                &mut registry,
                &mut policy,
                &inventory(),
                &catalogues(),
            ),
            ControlErrorCode::MalformedRequest,
        );
    }
    let mut old = request.clone();
    old["type"] = json!("register_logical_voices_v2");
    error(
        &process(
            &old.to_string(),
            &mut registry,
            &mut policy,
            &inventory(),
            &catalogues(),
        ),
        ControlErrorCode::MalformedRequest,
    );
    old["definitions"].as_array_mut().unwrap().remove(0);
    old["registry_generation"] = json!(42);
    assert!(matches!(
        process(
            &old.to_string(),
            &mut registry,
            &mut policy,
            &inventory(),
            &[]
        )
        .response,
        ControlResponse::LogicalVoicesRegisteredV2 { .. }
    ));
    assert!(!registry.is_engine_layered("bolden"));
    assert_eq!(registry.generation(), 42);
    error(
        &process(
            &request.to_string(),
            &mut registry,
            &mut policy,
            &inventory(),
            &catalogues(),
        ),
        ControlErrorCode::StaleGeneration,
    );
}

#[test]
fn oversized_native_acknowledgement_does_not_replace_the_registry() {
    let mut request = fixture("registration");
    let template = request["definitions"][0].clone();
    let mut definitions = Vec::new();
    for index in 0..36 {
        let mut definition = template.clone();
        definition["definition"]["id"] = json!(if index < 25 {
            format!("{index:0126}")
        } else {
            format!("{index:0125}")
        });
        definition["definition"]["choices"] =
            json!((0..32).map(|i| json!({
            "id":format!("c{i}"), "selector":{"kind":"engine_default","engine_id":"absent"},
            "adjustments":{}, "native":{"engine_id":"absent","schema_id":"s","parameters":{}}
        })).collect::<Vec<_>>());
        definitions.push(definition);
    }
    request["definitions"] = json!(definitions);
    let raw = request.to_string();
    assert!(raw.len() < MAX_CONTROL_PAYLOAD_BYTES);
    let mut registry = LogicalVoiceRegistry::default();
    let mut policy = RoutingPolicyRegistry::new("");
    process(
        &fixture("registration").to_string(),
        &mut registry,
        &mut policy,
        &inventory(),
        &catalogues(),
    );
    let saved = registry.registered_definitions().to_vec();
    request["registry_generation"] = json!(42);
    let envelope = decode_request(&STANDARD.encode(request.to_string())).unwrap();
    let ControlRequest::RegisterLogicalVoicesV3(body) = envelope.request else {
        panic!()
    };
    let internal = LogicalVoiceRegistry::default()
        .register_v3(body, &NativeCatalogueSnapshot::new(&[], &[]).unwrap())
        .unwrap();
    assert!(serde_json::to_vec(&internal).unwrap().len() <= MAX_CONTROL_PAYLOAD_BYTES);
    let result = process(&request.to_string(), &mut registry, &mut policy, &[], &[]);
    assert!(
        matches!(
            result.response,
            ControlResponse::Error {
                code: ControlErrorCode::PayloadTooLarge,
                ..
            }
        ),
        "{result:?}"
    );
    assert_eq!(registry.generation(), 41);
    assert_eq!(registry.registered_definitions(), saved);
}
