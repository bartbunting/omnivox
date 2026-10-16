use super::*;
use crate::timeline_v4::{decode_timeline_document, decode_timeline_v4};
use crate::timeline_v5::decode_timeline_v5;
use omnivox_core::tone_command::parse_rich_presentation_tone_arguments;
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/richer-tones.json"
    ))
    .unwrap()
}
fn encoded(value: &Value) -> String {
    STANDARD.encode(serde_json::to_vec(value).unwrap())
}

#[test]
fn standalone_and_timeline_preserve_the_same_sound_definition() {
    let fixture = fixture();
    let standalone =
        parse_rich_presentation_tone_arguments(fixture["standalone"].as_str().unwrap()).unwrap();
    let timeline = decode_timeline_v6(&encoded(&fixture["timeline"]), None).unwrap();
    let action = &timeline.actions[0];
    let PresentationAction::Tone {
        frequency_hz,
        duration_ms,
        volume,
        mode,
        ..
    } = &action.action.action
    else {
        panic!("tone was lost");
    };
    assert_eq!(
        ToneParameters {
            frequency_hz: *frequency_hz,
            duration_ms: *duration_ms,
            volume: *volume,
            shape: action.shape.unwrap()
        },
        standalone.tone
    );
    assert_eq!(*mode, PresentationAudioMode::Overlay);
    assert_eq!(
        decode_timeline_v6(&encode_timeline_v6(&timeline, false).unwrap(), None).unwrap(),
        timeline
    );
    assert_eq!(timeline.tracking_identity().unwrap().dispatch_id(), 91);
}

#[test]
fn old_decoders_reject_rich_shapes_and_dispatcher_selects_the_new_version() {
    let payload = encoded(&fixture()["timeline"]);
    assert!(decode_presentation_timeline(&payload).is_err());
    assert!(decode_timeline_v4(&payload, None).is_err());
    assert!(decode_timeline_v5(&payload, None).is_err());
    assert!(matches!(
        decode_timeline_document(&payload, None).unwrap(),
        crate::timeline_v4::TimelineDocument::Rich(_)
    ));
    let mut value = fixture()["timeline"].clone();
    value["protocol_version"] = json!(5);
    assert!(decode_timeline_v5(&encoded(&value), None).is_err());
    assert!(decode_timeline_v6(&encoded(&value), None).is_err());
    value["actions"][0].as_object_mut().unwrap().remove("shape");
    assert!(decode_timeline_v5(&encoded(&value), None).is_ok());
}

#[test]
fn classic_omission_preserves_legacy_frequency_range_without_inventing_a_shape() {
    let mut value = fixture()["timeline"].clone();
    value["actions"][0].as_object_mut().unwrap().remove("shape");
    value["actions"][0]["frequency_hz"] = json!(24000);
    value["actions"][0]["volume"] = json!(0);
    let timeline = decode_timeline_v6(&encoded(&value), None).unwrap();
    assert!(timeline.actions[0].shape.is_none());
    assert!(serde_json::to_value(&timeline).unwrap()["actions"][0]
        .get("shape")
        .is_none());
    value["actions"][0]["shape"] = json!({"waveform":"sine"});
    assert!(decode_timeline_v6(&encoded(&value), None).is_ok());
    value["actions"][0]["shape"]["waveform"] = json!("square");
    assert!(decode_timeline_v6(&encoded(&value), None).is_err());
}

#[test]
fn unknown_null_duplicate_and_misplaced_shapes_are_rejected() {
    for pointer in [
        "",
        "/spans/0",
        "/spans/0/span",
        "/actions/0",
        "/actions/0/position",
        "/actions/0/shape",
        "/actions/0/shape/envelope",
    ] {
        let mut value = fixture()["timeline"].clone();
        value.pointer_mut(pointer).unwrap()["unknown"] = json!(false);
        assert!(
            decode_timeline_v6(&encoded(&value), None).is_err(),
            "{pointer}"
        );
    }
    for pointer in [
        "/replacement_key",
        "/actions/0/shape",
        "/actions/0/shape/envelope",
        "/actions/0/shape/envelope/sustain_level",
    ] {
        let mut value = fixture()["timeline"].clone();
        *value.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(
            decode_timeline_v6(&encoded(&value), None).is_err(),
            "{pointer}"
        );
    }
    let raw = serde_json::to_string(&fixture()["timeline"])
        .unwrap()
        .replacen(
            "\"waveform\":\"triangle\"",
            "\"waveform\":\"triangle\",\"waveform\":\"square\"",
            1,
        );
    assert!(decode_timeline_v6(&STANDARD.encode(raw), None)
        .unwrap_err()
        .identity()
        .is_none());
    let mut value = fixture()["timeline"].clone();
    let action = value["actions"][0].as_object_mut().unwrap();
    for key in ["frequency_hz", "mode", "volume", "pan", "effect_bus"] {
        action.remove(key);
    }
    action.insert("type".into(), json!("silence"));
    assert!(decode_timeline_v6(&encoded(&value), None).is_err());
}

#[test]
fn invalid_values_and_cross_references_retain_tracking_identity() {
    for (pointer, invalid) in [
        ("/actions/0/frequency_hz", json!(20001)),
        ("/actions/0/frequency_hz", json!(0)),
        ("/actions/0/duration_ms", json!(0)),
        ("/actions/0/duration_ms", json!(60001)),
        ("/actions/0/volume", json!(1.01)),
        ("/actions/0/pan", json!(-0.1)),
        ("/actions/0/shape/envelope/attack_ms", json!(60001)),
        ("/actions/0/shape/envelope/decay_ms", json!(60001)),
        ("/actions/0/shape/envelope/release_ms", json!(60001)),
        ("/actions/0/shape/envelope/sustain_level", json!(1.01)),
        ("/actions/0/position/span_id", json!(999)),
        ("/actions/0/position/utf8_offset", json!(99)),
    ] {
        let mut value = fixture()["timeline"].clone();
        *value.pointer_mut(pointer).unwrap() = invalid;
        let error = decode_timeline_v6(&encoded(&value), None).unwrap_err();
        assert_eq!(error.identity().unwrap().dispatch_id(), 91, "{pointer}");
    }
}

#[test]
fn short_envelopes_zero_stages_and_each_waveform_remain_representable() {
    for waveform in ["sine", "triangle", "square"] {
        for time in [0, 60000] {
            let mut value = fixture()["timeline"].clone();
            let action = &mut value["actions"][0];
            action["duration_ms"] = json!(1);
            action["shape"]["waveform"] = json!(waveform);
            for stage in ["attack_ms", "decay_ms", "release_ms"] {
                action["shape"]["envelope"][stage] = json!(time);
            }
            assert!(decode_timeline_v6(&encoded(&value), None).is_ok());
        }
    }
}

#[test]
fn size_limits_exact_declarations_and_replacement_domains_are_preserved() {
    let value = fixture()["timeline"].clone();
    let bytes = serde_json::to_vec(&value).unwrap();
    let payload = STANDARD.encode(&bytes);
    let timeline = decode_timeline_v6(&payload, Some(bytes.len())).unwrap();
    assert!(decode_timeline_v6(&payload, Some(bytes.len() + 1)).is_err());
    assert!(decode_timeline_v6(&payload, Some(0)).is_err());
    assert!(decode_timeline_v6(&payload, Some(MAX_TIMELINE_AGGREGATE_BYTES + 1)).is_err());
    assert!(decode_timeline_v6(&"a".repeat(MAX_TIMELINE_ENCODED_BYTES + 1), None).is_err());
    assert!(timeline.shares_replacement_domain(&timeline));
    let mut other = timeline.clone();
    other.replacement_key = Some("another-domain".into());
    assert!(!timeline.shares_replacement_domain(&other));
    other = timeline.clone();
    other.protocol_version = 5;
    assert!(!timeline.shares_replacement_domain(&other));
    other = timeline.clone();
    other.delivery_policy = PresentationDeliveryPolicy::Ordered;
    assert!(!timeline.shares_replacement_domain(&other));
    let mut large = value;
    large["spans"][0]["span"]["text"] = json!("x".repeat(MAX_TIMELINE_PAYLOAD_BYTES));
    let bytes = serde_json::to_vec(&large).unwrap();
    let payload = STANDARD.encode(&bytes);
    assert!(decode_timeline_v6(&payload, None).is_err());
    let aggregate = decode_timeline_v6(&payload, Some(bytes.len())).unwrap();
    assert!(encode_timeline_v6(&aggregate, false).is_err());
    assert!(encode_timeline_v6(&aggregate, true).is_ok());
}

#[test]
fn programmatic_invalid_shapes_cannot_bypass_shared_validation() {
    let mut timeline = decode_timeline_v6(&encoded(&fixture()["timeline"]), None).unwrap();
    timeline.actions[0]
        .shape
        .as_mut()
        .unwrap()
        .envelope
        .as_mut()
        .unwrap()
        .sustain_level = f32::NAN;
    assert!(encode_timeline_v6(&timeline, false).is_err());
    timeline.actions[0].action.action = PresentationAction::SemanticEvent;
    assert!(timeline.validate(false).is_err());
    let mut timeline = decode_timeline_v6(&encoded(&fixture()["timeline"]), None).unwrap();
    timeline.actions[0].shape = None;
    if let PresentationAction::Tone { frequency_hz, .. } = &mut timeline.actions[0].action.action {
        *frequency_hz = f32::INFINITY;
    }
    assert!(encode_timeline_v6(&timeline, false).is_err());
}

#[test]
fn extended_fixture_preserves_standalone_and_timeline_parameters_and_domains() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/extended-tones.json"
    ))
    .unwrap();
    let standalone =
        parse_rich_presentation_tone_arguments(fixture["standalone"].as_str().unwrap()).unwrap();
    let mut value = fixture["timeline"].clone();
    let timeline = decode_timeline_v6(&encoded(&value), None).unwrap();
    assert_eq!(timeline.actions[0].shape, Some(standalone.tone.shape));
    assert!(matches!(
        decode_timeline_document(&encoded(&value), None).unwrap(),
        crate::timeline_v4::TimelineDocument::Rich(_)
    ));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        decode_timeline_document(&encoded(&value), Some(bytes.len()))
            .unwrap()
            .protocol_version(),
        7
    );
    let mut old = timeline.clone();
    old.protocol_version = 6;
    old.actions[0].shape = None;
    assert!(!timeline.shares_replacement_domain(&old));
    value["protocol_version"] = json!(6);
    assert!(decode_timeline_document(&encoded(&value), None).is_err());
    value["protocol_version"] = json!(7);
    for (key, bad) in [
        ("pulse_width", json!(0.99)),
        ("end_frequency_hz", json!(0)),
        ("envelope_curve", json!("unknown")),
        ("pulse_width", Value::Null),
    ] {
        let mut invalid = value.clone();
        invalid["actions"][0]["shape"][key] = bad;
        assert!(decode_timeline_document(&encoded(&invalid), None).is_err());
    }
}
