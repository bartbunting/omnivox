use super::*;
use crate::control::{decode_request, decode_response, ControlRequest};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
fn fixture(name: &str) -> Value {
    serde_json::from_str::<Value>(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()["messages"][name]
        .clone()
}
fn encoded(v: &Value) -> String {
    STANDARD.encode(serde_json::to_vec(v).unwrap())
}
#[test]
fn explanation_fixtures_roundtrip_and_validate_without_text() {
    let v = fixture("explain_request");
    let request = decode_request(&encoded(&v)).unwrap();
    let ControlRequest::ExplainVoiceParametersV1(r) = &request.request else {
        panic!()
    };
    r.source.validate(0.65).unwrap();
    assert!(r.source.validate(0.5).is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&serde_json::to_vec(&request).unwrap()).unwrap(),
        v
    );
    let v = fixture("explain_response");
    assert_eq!(
        serde_json::to_value(decode_response(&encoded(&v)).unwrap()).unwrap(),
        v
    );
    let applied = json!({"protocol_version":1,"request_id":5,"type":"explain_voice_parameters_v1", "source":{"mode":"applied","plan_id":"native-plan-1"}});
    let ControlRequest::ExplainVoiceParametersV1(r) =
        decode_request(&encoded(&applied)).unwrap().request
    else {
        panic!()
    };
    r.source.validate(0.5).unwrap();
}
#[test]
fn explanation_inputs_reject_unknown_missing_duplicate_and_automatic_selection() {
    for pointer in [
        "",
        "/source",
        "/source/voice",
        "/source/selection",
        "/source/voice/choices/0/native",
    ] {
        let mut v = fixture("explain_request");
        v.pointer_mut(pointer).unwrap()["unknown"] = json!(false);
        assert!(decode_request(&encoded(&v)).is_err(), "{pointer}");
    }
    let mut v = fixture("explain_request");
    v["source"]["text"] = json!("no synthesis");
    assert!(decode_request(&encoded(&v)).is_err());
    v = fixture("explain_request");
    v["source"]
        .as_object_mut()
        .unwrap()
        .remove("expected_base_rate");
    assert!(decode_request(&encoded(&v)).is_err());
    v = fixture("explain_request");
    v["source"]["selection"] = json!({"mode":"automatic"});
    let ControlRequest::ExplainVoiceParametersV1(r) = decode_request(&encoded(&v)).unwrap().request
    else {
        panic!()
    };
    assert!(r.source.validate(0.65).is_err());
    let raw = serde_json::to_string(&fixture("explain_request"))
        .unwrap()
        .replacen(
            "\"mode\":\"draft\"",
            "\"mode\":\"draft\",\"mode\":\"draft\"",
            1,
        );
    assert!(decode_request(&STANDARD.encode(raw)).is_err());
    for id in ["", "bad plan", &"p".repeat(129)] {
        assert!(ExplanationSource::Applied { plan_id: id.into() }
            .validate(0.5)
            .is_err());
    }
}
#[test]
fn explanation_responses_reject_fabricated_readback_and_invalid_evidence() {
    for (pointer, replacement) in [
        ("/result/parameters/0/read_back", json!(true)),
        ("/result/plan_id", json!("invented")),
        ("/result/choice_id", json!("")),
        ("/result/identity/runtime_generation", json!(0)),
        ("/result/parameters/0/masked_native", json!(true)),
    ] {
        let mut v = fixture("explain_response");
        *v.pointer_mut(pointer).unwrap() = replacement;
        assert!(decode_response(&encoded(&v)).is_err(), "{pointer}");
    }
    let mut v = fixture("explain_response");
    v["result"]["unknown"] = json!(true);
    assert!(decode_response(&encoded(&v)).is_err());
    let raw = serde_json::to_string(&fixture("explain_response"))
        .unwrap()
        .replacen(
            "\"read_back\":false",
            "\"read_back\":false,\"read_back\":false",
            1,
        );
    assert!(decode_response(&STANDARD.encode(raw)).is_err());
    let mut v = fixture("explain_response");
    v["result"]["parameters"] = json!(vec![v["result"]["parameters"][0].clone(); 65]);
    assert!(decode_response(&encoded(&v)).is_err());
}
