use std::io::Cursor;

use serde_json::{json, Value};

use super::*;

fn request(version: u16) -> Value {
    let mut value = json!({"protocol_version":version,"request_id":9,"type":"synthesize",
        "text":"hello","settings":{"voice_id":null,"rate":0.5,"pitch":1.0,"volume":1.0}});
    if version >= 2 {
        value["anchors"] = json!([]);
    }
    if version >= 3 {
        for key in ["pitch_range", "stress", "richness"] {
            value["settings"][key] = Value::Null;
        }
    }
    value
}

fn read<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, HelperProtocolError> {
    read_frame(&mut Cursor::new(encode_frame(value).unwrap())).map(Option::unwrap)
}

#[test]
fn legacy_requests_cannot_silently_drop_native_parameters() {
    for version in 1..=5 {
        let baseline = request(version);
        read::<HelperRequest>(&baseline)
            .unwrap()
            .validate()
            .unwrap();
        for native in [
            Value::Null,
            json!({"native":{"engine_id":"dectalk","schema_id":"dectalk.design-voice.v1","parameters":{"sm":{"op":"set","value":55}}}}),
        ] {
            let mut value = baseline.clone();
            value["voice_parameters"] = native;
            assert!(
                read::<HelperRequest>(&value).is_err(),
                "version {version}: {value}"
            );
        }
        let mut value = baseline;
        value["settings"]["native"] = json!({"sm":55});
        assert!(read::<HelperRequest>(&value).is_err());
    }
}

#[test]
fn all_legacy_operation_shapes_reject_extra_members() {
    for version in 1..=5 {
        for body in [
            json!({"type":"hello","supported_protocol_versions":[version]}),
            json!({"type":"describe"}),
            json!({"type":"ping"}),
            json!({"type":"shutdown"}),
            json!({"type":"cancel","target_request_id":2}),
        ] {
            let mut value = body;
            value["protocol_version"] = version.into();
            value["request_id"] = 3.into();
            read::<HelperRequest>(&value).unwrap().validate().unwrap();
            value["voice_parameters"] = Value::Null;
            assert!(read::<HelperRequest>(&value).is_err(), "{value}");
        }
    }
}

#[test]
fn version_specific_fields_are_rejected_even_when_null() {
    let mut value = request(1);
    value["anchors"] = Value::Null;
    assert!(read::<HelperRequest>(&value).is_err());
    for version in [1, 2] {
        for field in ["pitch_range", "stress", "richness"] {
            let mut value = request(version);
            value["settings"][field] = Value::Null;
            assert!(read::<HelperRequest>(&value).is_err());
        }
    }
    let mut value = request(5);
    value["anchors"] = json!([{"id":"a","text_offset":0,"affinity":"before","native":null}]);
    assert!(read::<HelperRequest>(&value).is_err());
    for version in 1..5 {
        let value = json!({"protocol_version":version,"request_id":9,"type":"markers",
            "markers":[{"kind":"word","frame_offset":0,"value":"hello","text_start":0,"text_length":5,"resolution":null}]});
        assert!(read::<HelperResponse>(&value).is_err());
    }
}

#[test]
fn raw_duplicate_members_are_rejected_at_every_depth() {
    for raw in [
        r#"{"protocol_version":5,"request_id":9,"type":"ping","type":"shutdown"}"#,
        r#"{"protocol_version":5,"request_id":9,"request_id":10,"type":"ping"}"#,
        r#"{"protocol_version":5,"request_id":9,"type":"synthesize","text":"hello","settings":{"rate":0.5,"rate":0.9,"pitch":1,"volume":1},"anchors":[]}"#,
        r#"{"protocol_version":5,"request_id":9,"type":"synthesize","text":"hello","settings":{"rate":0.5,"\u0072ate":0.9,"pitch":1,"volume":1},"anchors":[]}"#,
        r#"{"protocol_version":5,"request_id":9,"type":"synthesize","text":"hello","settings":{"rate":0.5,"pitch":1,"volume":1},"anchors":[{"id":"a","id":"b","text_offset":0,"affinity":"before"}]}"#,
    ] {
        assert!(serde_json::from_str::<HelperRequest>(raw).is_err(), "{raw}");
        assert!(read_frame::<_, HelperRequest>(&mut Cursor::new(format!("{raw}\n"))).is_err());
    }
    for raw in [
        r#"{"protocol_version":5,"request_id":9,"type":"synthesis_started","actual_voice_id":"paul","format":{"sample_rate":11025,"sample_rate":22050,"channels":1,"sample_format":"pcm_s16_le"}}"#,
        r#"{"protocol_version":5,"request_id":9,"type":"markers","markers":[{"kind":"word","kind":"sentence","frame_offset":0}]}"#,
        r#"{"protocol_version":5,"request_id":9,"type":"descriptor","descriptor":{"unknown":{"a":1,"a":2}}}"#,
    ] {
        assert!(
            serde_json::from_str::<HelperResponse>(raw).is_err(),
            "{raw}"
        );
    }
}

#[test]
fn legacy_responses_reject_native_receipts_and_unknown_payload_fields() {
    for version in 1..=5 {
        let baseline = json!({"protocol_version":version,"request_id":9,"type":"synthesis_started",
            "actual_voice_id":"paul","format":{"sample_rate":11025,"channels":1,"sample_format":"pcm_s16_le"}});
        read::<HelperResponse>(&baseline)
            .unwrap()
            .validate()
            .unwrap();
        let mut value = baseline.clone();
        value["native_application"] = Value::Null;
        assert!(read::<HelperResponse>(&value).is_err());
        let mut value = baseline;
        value["format"]["native_application"] = Value::Null;
        assert!(read::<HelperResponse>(&value).is_err());
        for body in [
            json!({"type":"pong"}),
            json!({"type":"synthesis_completed","frame_count":0}),
            json!({"type":"synthesis_cancelled"}),
            json!({"type":"shutting_down"}),
            json!({"type":"cancel_accepted","target_request_id":2}),
            json!({"type":"error","code":"invalid_request","message":"bad request","retryable":false}),
        ] {
            let mut value = body;
            value["protocol_version"] = version.into();
            value["request_id"] = 9.into();
            read::<HelperResponse>(&value).unwrap().validate().unwrap();
            value["native_application"] = Value::Null;
            assert!(read::<HelperResponse>(&value).is_err(), "{value}");
        }
    }
}

#[test]
fn escaping_repeated_values_and_legacy_frames_remain_valid() {
    for version in 1..=5 {
        let mut value = request(version);
        value["text"] = json!("\"rate\":1,\"rate\":2; {braces} [arrays] \\ \n λ 🍕");
        if version >= 2 {
            value["anchors"] = json!([
            {"id":"a","text_offset":0,"affinity":"before"},
            {"id":"b","text_offset":0,"affinity":"before"}]);
        }
        let decoded: HelperRequest = read(&value).unwrap();
        decoded.validate().unwrap();
        let encoded = encode_frame(&decoded).unwrap();
        assert_eq!(
            decoded,
            read_frame(&mut Cursor::new(encoded)).unwrap().unwrap()
        );
    }
    let escaped = r#"{"protocol_version":5,"request_id":9,"t\u0079pe":"ping"}"#;
    serde_json::from_str::<HelperRequest>(escaped)
        .unwrap()
        .validate()
        .unwrap();
    let error = json!({"protocol_version":5,"request_id":null,"type":"error","code":"invalid_request","message":"bad frame","retryable":false});
    read::<HelperResponse>(&error).unwrap().validate().unwrap();
}

#[test]
fn bounded_frames_and_future_negotiation_remain_unchanged() {
    assert_eq!(SUPPORTED_HELPER_PROTOCOL_VERSIONS, &[5, 4, 3, 2, 1]);
    let future = json!({"protocol_version":6,"request_id":1,"type":"hello","supported_protocol_versions":[6,5]});
    assert!(matches!(
        read::<HelperRequest>(&future).unwrap().validate(),
        Err(HelperProtocolError::UnsupportedVersion(6))
    ));
    let mut value = request(5);
    value["text"] = "x".repeat(MAX_HELPER_FRAME_BYTES).into();
    let raw = format!("{value}\n");
    assert!(matches!(
        read_frame::<_, HelperRequest>(&mut Cursor::new(raw)),
        Err(HelperProtocolError::FrameTooLarge)
    ));
}

#[test]
fn legacy_unowned_errors_may_omit_request_id() {
    for version in 1..=5 {
        let error = json!({"protocol_version":version,"type":"error","code":"invalid_request",
            "message":"bad frame","retryable":false});
        let response: HelperResponse = read(&error).unwrap();
        assert_eq!(response.request_id, None);
        response.validate().unwrap();
        let request = json!({"protocol_version":version,"type":"ping"});
        assert!(read::<HelperRequest>(&request).is_err());
    }
}
