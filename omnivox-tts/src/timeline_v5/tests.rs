use super::*;
use crate::timeline_v4::{decode_timeline_document, decode_timeline_v4, TimelineDocument};
use serde_json::{json, Value};
fn fixture() -> Value {
    serde_json::from_str::<Value>(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()["messages"]["timeline"]
        .clone()
}
fn encoded(value: &Value) -> String {
    STANDARD.encode(serde_json::to_vec(value).unwrap())
}
#[test]
fn native_timeline_fixture_preserves_context_and_version_boundaries() {
    let value = fixture();
    let payload = encoded(&value);
    let timeline = decode_timeline_v5(&payload, None).unwrap();
    assert_eq!(serde_json::to_value(&timeline).unwrap(), value);
    assert_eq!(
        decode_timeline_v5(&encode_timeline_v5(&timeline, false).unwrap(), None).unwrap(),
        timeline
    );
    assert!(matches!(
        decode_timeline_document(&payload, None).unwrap(),
        TimelineDocument::Native(_)
    ));
    assert!(decode_presentation_timeline(&payload).is_err());
    assert!(decode_timeline_v4(&payload, None).is_err());
    let mut old = value;
    old["protocol_version"] = json!(4);
    assert!(decode_timeline_document(&encoded(&old), None).is_err());
    let MixedSpeechSpan::EngineLayered(span) = &timeline.into_execution().spans[0] else {
        panic!("mode lost")
    };
    assert!(span.context.richness.is_some());
    assert!(span.context.rate_offset.is_some());
}
#[test]
fn native_timeline_rejects_unknown_null_duplicate_and_mixed_fields() {
    for (pointer, extra) in [
        ("", "extra"),
        ("/spans/0", "extra"),
        ("/spans/0/span", "acss"),
        ("/spans/0/span/context", "extra"),
        ("/spans/0/span/context/richness", "extra"),
        ("/spans/0/span/placement", "extra"),
    ] {
        let mut value = fixture();
        value.pointer_mut(pointer).unwrap()[extra] = json!(0);
        assert!(
            decode_timeline_v5(&encoded(&value), None).is_err(),
            "{pointer}"
        );
    }
    let mut value = fixture();
    value["replacement_key"] = Value::Null;
    assert!(decode_timeline_v5(&encoded(&value), None).is_err());
    let raw = serde_json::to_string(&fixture()).unwrap().replacen(
        "\"protocol_version\":5",
        "\"protocol_version\":5,\"protocol_version\":5",
        1,
    );
    assert!(decode_timeline_v5(&STANDARD.encode(raw), None)
        .unwrap_err()
        .identity()
        .is_none());
    let mut invalid = fixture();
    invalid["spans"][0]["span"]["id"] = json!(0);
    assert!(decode_timeline_v5(&encoded(&invalid), None)
        .unwrap_err()
        .identity()
        .is_some());
}
#[test]
fn native_legacy_members_and_actions_reject_unknown_nested_fields() {
    let mut value = fixture();
    value["spans"] = json!([{"mode":"legacy","span":{"id":1,"text":"note","acss":{},"effects":{"mode":"replace","state_id":"x","style":{}}}}]);
    value["actions"] = json!([{"id":"a","position":{"position":"span_boundary","span_id":1,"affinity":"after"},"lifecycle_anchor":"run","type":"semantic_event"}]);
    decode_timeline_v5(&encoded(&value), None).unwrap();
    for pointer in [
        "/spans/0/span/acss",
        "/spans/0/span/effects",
        "/spans/0/span/effects/style",
        "/actions/0",
        "/actions/0/position",
    ] {
        let mut invalid = value.clone();
        invalid.pointer_mut(pointer).unwrap()["extra"] = json!(false);
        assert!(
            decode_timeline_v5(&encoded(&invalid), None).is_err(),
            "{pointer}"
        );
    }
}
#[test]
fn native_multipart_declaration_is_exact_and_replacement_is_version_local() {
    let value = fixture();
    let bytes = serde_json::to_vec(&value).unwrap();
    let payload = STANDARD.encode(&bytes);
    decode_timeline_v5(&payload, Some(bytes.len())).unwrap();
    assert!(decode_timeline_v5(&payload, Some(bytes.len() + 1)).is_err());
    let header = format!("5 8 91 0 1 {} {payload}", bytes.len());
    decode_any_timeline_part(&header).unwrap();
    assert!(decode_presentation_timeline_part(&header).is_err());
    let mut timeline = decode_timeline_v5(&payload, None).unwrap();
    timeline.delivery_policy = PresentationDeliveryPolicy::Replaceable;
    timeline.replacement_key = Some("navigation".into());
    assert!(timeline.shares_replacement_domain(&timeline));
    assert!(!TimelineDocument::Native(timeline.clone())
        .shares_replacement_domain(&TimelineDocument::Layered(timeline.into_execution())));
}

#[test]
fn native_registry_requires_exact_generation_and_definition_mode() {
    use crate::engine_voice_choices::{NativeCatalogueSnapshot, VoiceRegistrationV3};
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    let mut value = fixture["messages"]["registration"].clone();
    for key in ["protocol_version", "request_id", "type"] {
        value.as_object_mut().unwrap().remove(key);
    }
    let registration: VoiceRegistrationV3 = serde_json::from_value(value).unwrap();
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v3(
            registration,
            &NativeCatalogueSnapshot::new(&[], &[]).unwrap(),
        )
        .unwrap();
    let mut timeline =
        decode_timeline_v5(&encoded(&fixture["messages"]["timeline"]), None).unwrap();
    timeline.validate_registry(&registry).unwrap();
    timeline.registry_generation += 1;
    assert!(timeline.validate_registry(&registry).is_err());
    timeline.registry_generation -= 1;
    let NativeSpeechSpan::EngineLayered(span) = timeline.spans[0].clone() else {
        unreachable!()
    };
    timeline.spans[0] = NativeSpeechSpan::Layered(span);
    assert!(timeline.validate_registry(&registry).is_err());
    timeline.spans[0] = NativeSpeechSpan::Legacy(PresentationSpeechSpan {
        id: 1,
        text: "note".into(),
        logical_voice_id: Some("bolden".into()),
        acss: Default::default(),
        rate_offset: None,
        effects: Default::default(),
    });
    assert!(timeline.validate_registry(&registry).is_err());
}
