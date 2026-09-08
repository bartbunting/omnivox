use super::*;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../docs/protocol-fixtures/voice-choice-tuning.json"
    ))
    .unwrap()
}

fn process(raw: &str, registry: &mut LogicalVoiceRegistry) -> ControlResponseEnvelope {
    process_control_request(
        &STANDARD.encode(raw),
        "test",
        12,
        "",
        &[],
        &[],
        registry,
        &mut RoutingPolicyRegistry::new(""),
    )
}

#[test]
fn registration_fixture_roundtrips_and_returns_bounded_flat_acknowledgement() {
    let examples = fixture();
    let registration = &examples["messages"]["registration"];
    let encoded = STANDARD.encode(registration.to_string());
    let request = decode_request(&encoded).unwrap();
    assert_eq!(
        decode_request(&encode_request(&request).unwrap()).unwrap(),
        request
    );
    let ack: ControlResponseEnvelope =
        serde_json::from_value(examples["messages"]["registration_ack"].clone()).unwrap();
    assert_eq!(
        decode_response(&encode_response(&ack).unwrap()).unwrap(),
        ack
    );

    let mut registry = LogicalVoiceRegistry::default();
    let response = process(&registration.to_string(), &mut registry);
    assert_eq!(response.request_id, Some(701));
    assert_eq!(registry.generation(), 41);
    let mut expected = examples["messages"]["registration_ack"].clone();
    // Empty inventory keeps definitions while reporting both unresolved IDs.
    expected["unresolved_logical_voice_ids"] = json!(registration["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["definition"]["id"].as_str().unwrap())
        .collect::<Vec<_>>());
    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
    let retry = process(&registration.to_string(), &mut registry);
    assert_eq!(response, retry);
    let capabilities = process(
        r#"{"protocol_version":1,"request_id":2,"type":"capabilities"}"#,
        &mut registry,
    );
    let ControlResponse::Capabilities { features, .. } = capabilities.response else {
        panic!("capabilities")
    };
    for feature in [
        "voice_choice_tuning_v1",
        "presentation_timeline_v4",
        "playback_marker_events_v3",
    ] {
        assert!(
            features.iter().any(|f| f == feature),
            "complete bundle is missing: {feature}"
        );
    }
}

#[test]
fn malformed_new_forms_and_patch_bounds_leave_the_registry_unchanged() {
    let original = fixture()["messages"]["registration"].clone();
    let mut registry = LogicalVoiceRegistry::default();
    process(&original.to_string(), &mut registry);
    let definitions = registry.registered_definitions().to_vec();
    let mut cases = Vec::new();
    for pointer in [
        "/fallback_policy/global_default",
        "/fallback_policy/fallback_engines",
        "/definitions/0/definition/shared/rate_offset",
        "/definitions/0/definition/shared/acss/volume",
    ] {
        let mut changed = original.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        changed
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        cases.push(changed);
    }
    for pointer in [
        "",
        "/fallback_policy",
        "/definitions/0",
        "/definitions/0/definition",
        "/definitions/0/definition/choices/0/selector",
        "/definitions/0/definition/shared/effects",
    ] {
        let mut changed = original.clone();
        changed.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
        cases.push(changed);
    }
    for patch in [
        json!({"gain":null}),
        json!({"gain":{"op":"set","value":1.01}}),
        json!({"rate_offset":{"op":"set","value":21}}),
        json!({"gain":{"op":"default","value":0}}),
    ] {
        let mut changed = original.clone();
        changed["definitions"][0]["definition"]["choices"][0]["adjustments"] = patch;
        cases.push(changed);
    }
    let mut unknown_global = original.clone();
    unknown_global["fallback_policy"]["global_default"] =
        json!({"kind":"engine_default","engine_id":"espeak","unknown":true});
    cases.push(unknown_global);
    for mut changed in cases {
        changed["registry_generation"] = json!(42);
        let response = process(&changed.to_string(), &mut registry);
        assert!(
            matches!(response.response, ControlResponse::Error { .. }),
            "accepted {changed}"
        );
        assert_eq!(response.request_id, Some(701));
        assert_eq!(registry.generation(), 41);
        assert_eq!(registry.registered_definitions(), definitions);
    }
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth_and_identity_is_unambiguous() {
    let raw = fixture()["messages"]["registration"].to_string();
    for (needle, replacement, identity) in [
        (
            "\"request_id\":701",
            "\"request_id\":701,\"request_id\":702",
            None,
        ),
        (
            "\"registry_generation\":41",
            "\"registry_generation\":41,\"registry_generation\":42",
            Some(701),
        ),
        (
            "\"op\":\"set\"",
            "\"op\":\"set\",\"op\":\"default\"",
            Some(701),
        ),
        (
            "\"preferred_engines\":[]",
            "\"preferred_engines\":[],\"preferred_engines\":[]",
            Some(701),
        ),
        (
            "\"id\":\"bolden\"",
            "\"id\":\"bolden\",\"id\":\"bolden\"",
            Some(701),
        ),
        (
            "\"type\":\"register_logical_voices_v2\"",
            "\"type\":\"register_logical_voices_v2\",\"type\":\"inventory\"",
            Some(701),
        ),
    ] {
        assert!(raw.contains(needle));
        let mut registry = LogicalVoiceRegistry::default();
        let response = process(&raw.replacen(needle, replacement, 1), &mut registry);
        assert!(
            matches!(
                response.response,
                ControlResponse::Error {
                    code: ControlErrorCode::MalformedRequest,
                    ..
                }
            ),
            "{response:?}"
        );
        assert_eq!(response.request_id, identity);
        assert_eq!(registry.generation(), 0);
    }
    // Legacy extension data remains accepted by the old operation. The new
    // envelope retains extensions but requires all their nested keys be unique.
    let legacy = r#"{"id":"plain","preferences":[],"acss":{},"extension":{"x":1,"x":2}}"#;
    let old = format!(
        r#"{{"protocol_version":1,"request_id":1,"type":"register_logical_voices","registry_generation":1,"definitions":[{legacy}]}}"#
    );
    assert!(decode_request(&STANDARD.encode(old)).is_ok());
    let new = format!(
        r#"{{"protocol_version":1,"request_id":1,"type":"register_logical_voices_v2","registry_generation":1,"definitions":[{{"mode":"legacy","definition":{legacy}}}],"fallback_policy":{{"preferred_engines":[],"allow_same_language_on_requested_engine":false,"global_default":null,"fallback_engines":[]}}}}"#
    );
    assert!(decode_request(&STANDARD.encode(&new)).is_err());
    assert!(decode_request(&STANDARD.encode(new.replace("\"x\":1,\"x\":2", "\"x\":1"))).is_ok());
}

#[test]
fn stale_conflict_and_zero_generations_are_reported_without_publishing() {
    let mut raw = fixture()["messages"]["registration"].clone();
    let mut registry = LogicalVoiceRegistry::default();
    process(&raw.to_string(), &mut registry);
    let definitions = registry.registered_definitions().to_vec();
    for (generation, code) in [
        (40, ControlErrorCode::StaleGeneration),
        (0, ControlErrorCode::InvalidConfiguration),
        (41, ControlErrorCode::GenerationConflict),
    ] {
        raw["registry_generation"] = json!(generation);
        raw["definitions"][0]["definition"]["choices"][0]["id"] = json!("different-row");
        let response = process(&raw.to_string(), &mut registry);
        assert!(
            matches!(response.response, ControlResponse::Error { code: actual, .. } if actual == code)
        );
        assert_eq!(registry.generation(), 41);
        assert_eq!(registry.registered_definitions(), definitions);
    }
}

#[test]
fn maximum_registry_ack_and_utf8_errors_fit_their_bounds() {
    let mut raw = fixture()["messages"]["registration"].clone();
    raw["definitions"] =
        json!((0..crate::logical_voices::MAX_LOGICAL_VOICES).map(|index| {
        json!({"mode":"legacy","definition":{"id":format!("{index:0128}"),"preferences":[],"acss":{}}})
    }).collect::<Vec<_>>());
    let response = process(&raw.to_string(), &mut LogicalVoiceRegistry::default());
    let ControlResponse::LogicalVoicesRegisteredV2 {
        definition_count,
        ref unresolved_logical_voice_ids,
        ..
    } = response.response
    else {
        panic!("{response:?}")
    };
    assert_eq!(definition_count, 256);
    assert_eq!(unresolved_logical_voice_ids.len(), 256);
    let wire = format_control_event(&response).unwrap();
    assert!(wire.len() < 512 * 1024);
    assert_eq!(
        decode_response(wire.split_once(' ').unwrap().1).unwrap(),
        response
    );
    let diagnostic = bounded_message("音".repeat(1000));
    assert_eq!(diagnostic.len(), 1023);
    assert!(diagnostic.chars().all(|character| character == '音'));
}
