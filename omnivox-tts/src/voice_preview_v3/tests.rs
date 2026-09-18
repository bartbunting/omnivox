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
fn encoded(value: &Value) -> String {
    STANDARD.encode(serde_json::to_vec(value).unwrap())
}
#[test]
fn native_preview_fixtures_roundtrip_with_explicit_selection_and_receipts() {
    let value = fixture("preview");
    let decoded = decode_request(&encoded(&value)).unwrap();
    let ControlRequest::PreviewVoiceV3(request) = &decoded.request else {
        panic!("wrong version")
    };
    assert_eq!(request.validate(0.65).unwrap(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&serde_json::to_vec(&decoded).unwrap()).unwrap(),
        value
    );
    let value = fixture("preview_completed");
    let decoded = decode_response(&encoded(&value)).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&serde_json::to_vec(&decoded).unwrap()).unwrap(),
        value
    );
}
#[test]
fn native_preview_rejects_missing_unknown_duplicate_and_old_format_members() {
    for pointer in [
        "",
        "/voice",
        "/voice/choices/0",
        "/voice/choices/0/native",
        "/context",
        "/selection",
        "/placement",
    ] {
        let mut value = fixture("preview");
        value.pointer_mut(pointer).unwrap()["unknown"] = json!(false);
        assert!(decode_request(&encoded(&value)).is_err(), "{pointer}");
    }
    let mut missing = fixture("preview");
    missing["voice"]["choices"][0]
        .as_object_mut()
        .unwrap()
        .remove("native");
    assert!(decode_request(&encoded(&missing)).is_err());
    let mut old = fixture("preview");
    old["type"] = json!("preview_voice_v2");
    assert!(decode_request(&encoded(&old)).is_err());
    let raw = serde_json::to_string(&fixture("preview"))
        .unwrap()
        .replacen("\"text\":", "\"text\":\"duplicate\",\"text\":", 1);
    assert!(decode_request(&STANDARD.encode(raw)).is_err());
    let mut missing = fixture("preview_completed");
    missing["last_started"]
        .as_object_mut()
        .unwrap()
        .remove("native_application");
    assert!(decode_response(&encoded(&missing)).is_err());
}
#[test]
fn native_preview_preserves_common_bounds_and_selection_validation() {
    let ControlRequest::PreviewVoiceV3(mut request) = decode_request(&encoded(&fixture("preview")))
        .unwrap()
        .request
    else {
        unreachable!()
    };
    assert!(request.validate(0.66).is_err());
    request.selection = VoicePreviewSelection::Choice {
        choice_id: "missing".into(),
    };
    assert!(request.validate(0.65).is_err());
    request.selection = VoicePreviewSelection::Automatic {};
    request.text = "x".repeat(crate::control::MAX_PREVIEW_TEXT_BYTES + 1);
    assert!(request.validate(0.65).is_err());
    request.text = "sample".into();
    let choice = request.voice.choices[0].clone();
    request.voice.choices = (0..33)
        .map(|i| {
            let mut c = choice.clone();
            c.id = format!("c{i}");
            c
        })
        .collect();
    assert!(request.validate(0.65).is_err());
}
#[test]
fn native_terminal_truncation_preserves_last_started_and_valid_utf8() {
    let ControlResponse::PreviewVoiceCompletedV3(mut response) =
        decode_response(&encoded(&fixture("preview_completed")))
            .unwrap()
            .response
    else {
        unreachable!()
    };
    let mut identity = response.last_started.clone().unwrap();
    identity.realized.voice_id = "v".repeat(20_000);
    identity.validate().unwrap();
    response.last_started = Some(identity.clone());
    response.accepted_audio = (0..35).map(|_| (identity.clone(), false).into()).collect();
    response.message = Some("é".repeat(2000));
    let record = response.bounded_event(802).unwrap();
    let ControlResponse::PreviewVoiceCompletedV3(response) =
        decode_response(record.split_whitespace().last().unwrap())
            .unwrap()
            .response
    else {
        unreachable!()
    };
    assert!(response.accepted_audio_truncated);
    assert!(response.accepted_audio.len() < 32);
    assert_eq!(response.last_started, Some(identity));
    assert!(response.accepted_audio.iter().all(|a| !a.playback_started));
    assert!(response.message.unwrap().len() <= MAX_PREVIEW_MESSAGE_BYTES);
}

#[test]
fn native_preview_reply_rejects_duplicate_and_invalid_application_evidence() {
    let raw = serde_json::to_string(&fixture("preview_completed"))
        .unwrap()
        .replacen(
            "\"status\":\"completed\"",
            "\"status\":\"completed\",\"status\":\"completed\"",
            1,
        );
    assert!(decode_response(&STANDARD.encode(raw)).is_err());
    let mut value = fixture("preview_completed");
    value["last_started"]["native_application"]["plan_id"] = Value::Null;
    assert!(decode_response(&encoded(&value)).is_err());
    let mut value = fixture("preview_completed");
    value["accepted_audio"][0]["native_application"]["identity"]["runtime_generation"] = json!(0);
    assert!(decode_response(&encoded(&value)).is_err());
}
