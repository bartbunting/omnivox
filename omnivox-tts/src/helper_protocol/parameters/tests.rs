use super::super::{
    encode_frame, read_frame, HelperRequest, HelperResponse, SUPPORTED_HELPER_PROTOCOL_VERSIONS,
};
use super::*;
use serde_json::{json, Value};
use std::io::Cursor;

fn fixture(name: &str) -> Value {
    let all: Value = serde_json::from_str(include_str!(
        "../../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    all["messages"][name].clone()
}
fn request(v: &Value) -> Result<Request, HelperProtocolError> {
    read_frame(&mut Cursor::new(encode_frame(v)?)).map(Option::unwrap)
}
fn response(v: &Value) -> Result<Response, HelperProtocolError> {
    read_frame(&mut Cursor::new(encode_frame(v)?)).map(Option::unwrap)
}
fn catalogue() -> Value {
    let mut v = fixture("catalogue_response");
    v["protocol_version"] = 6.into();
    v
}
fn query() -> Value {
    let mut v = fixture("catalogue_request");
    v["protocol_version"] = 6.into();
    v
}
fn explanation(name: &str) -> Value {
    let mut v = fixture(name);
    v["protocol_version"] = 6.into();
    if let Some(o) = v["result"].as_object_mut() {
        o.remove("choice_id");
    }
    v
}

#[test]
fn independent_helper_fixtures_roundtrip() {
    for name in ["helper_synthesis", "helper_explain_request"] {
        let v = fixture(name);
        let decoded = request(&v).unwrap();
        let roundtrip: Request = read_frame(&mut Cursor::new(encode_frame(&decoded).unwrap()))
            .unwrap()
            .unwrap();
        assert_eq!(roundtrip, decoded);
        // Nullable optional common settings retain their legacy serialization.
        let mut normalized = v;
        if name == "helper_synthesis" {
            normalized["settings"]
                .as_object_mut()
                .unwrap()
                .retain(|_, v| !v.is_null());
        } else {
            normalized["source"]["settings"]
                .as_object_mut()
                .unwrap()
                .retain(|_, v| !v.is_null());
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&encode_frame(&decoded).unwrap()).unwrap(),
            normalized
        );
    }
    for v in [
        catalogue(),
        explanation("explain_response"),
        explanation("applied_explanation"),
        explanation("expired_explanation"),
        fixture("helper_synthesis_started"),
    ] {
        let decoded = response(&v).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), v);
    }
    request(&query()).unwrap();
    let req = request(&fixture("helper_synthesis")).unwrap();
    response(&fixture("helper_synthesis_started"))
        .unwrap()
        .validate_for(&req, "dectalk")
        .unwrap();
}

#[test]
fn required_nulls_cannot_be_omitted_or_downgraded() {
    let mut synth = fixture("helper_synthesis");
    synth["voice_parameters"] = Value::Null;
    request(&synth).unwrap();
    synth.as_object_mut().unwrap().remove("voice_parameters");
    assert!(request(&synth).is_err());
    let mut started = fixture("helper_synthesis_started");
    started["native_application"] = Value::Null;
    response(&started).unwrap();
    started
        .as_object_mut()
        .unwrap()
        .remove("native_application");
    assert!(response(&started).is_err());
    for field in ["voice_id", "cursor", "expected_catalogue_revision"] {
        let mut q = query();
        q.as_object_mut().unwrap().remove(field);
        assert!(request(&q).is_err());
    }
    for version in 1..=5 {
        let mut v = fixture("helper_synthesis");
        v["protocol_version"] = version.into();
        assert!(request(&v).is_err());
        assert!(serde_json::from_value::<HelperRequest>(v).is_err());
        let mut v = fixture("helper_synthesis_started");
        v["protocol_version"] = version.into();
        assert!(response(&v).is_err());
        assert!(serde_json::from_value::<HelperResponse>(v).is_err());
    }
    assert_eq!(SUPPORTED_HELPER_PROTOCOL_VERSIONS, &[5, 4, 3, 2, 1]);
    let old_hello: HelperRequest = serde_json::from_value(json!({"protocol_version":6,"request_id":1,"type":"hello","supported_protocol_versions":[6]})).unwrap();
    assert!(old_hello.validate().is_err());
}

#[test]
fn duplicate_unknown_and_mixed_shapes_are_rejected() {
    let raw = fixture("helper_synthesis").to_string();
    for modified in [
        raw.replace("\"request_id\":3", "\"request_id\":3,\"request_id\":4"),
        raw.replace("\"sm\":", "\"sm\":{\"op\":\"default\"},\"\\u0073m\":"),
        raw.replace("\"richness\":0.5", "\"richness\":0.5,\"richness\":0.5"),
    ] {
        assert_ne!(modified, raw);
        assert!(serde_json::from_str::<Request>(&modified).is_err());
    }
    for pointer in [
        "",
        "/settings",
        "/voice_parameters",
        "/voice_parameters/native",
        "/voice_parameters/expected_identity",
        "/voice_parameters/native/parameters/sm",
    ] {
        let mut v = fixture("helper_synthesis");
        v.pointer_mut(pointer).unwrap()["unexpected"] = Value::Null;
        assert!(request(&v).is_err(), "{pointer}");
    }
    let mut v = fixture("helper_synthesis");
    v["anchors"] = json!([{"id":"a","text_offset":0,"affinity":"before","unexpected":null}]);
    assert!(request(&v).is_err());
    let mut v = fixture("helper_explain_request");
    v["source"]["plan_id"] = "p1".into();
    assert!(request(&v).is_err());
    let mut v = explanation("applied_explanation");
    v["result"]["realized"]["unexpected"] = Value::Null;
    assert!(response(&v).is_err());
    let mut v = catalogue();
    v["result"]["parameters"][0]["value_type"] = json!({"kind":"boolean","maximum":1});
    v["result"]["parameters"][0]["default"]["value"] = false.into();
    assert!(response(&v).is_err());
}

#[test]
fn sparse_native_values_and_context_preserve_meaning() {
    let mut v = fixture("helper_synthesis");
    v["voice_parameters"]["native"]["parameters"] = json!({"sm":{"op":"set","value":0},"flag":{"op":"set","value":false},"ri":{"op":"default"}});
    let r = request(&v).unwrap();
    let RequestBody::Synthesize {
        voice_parameters: Some(p),
        ..
    } = r.body
    else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(p.native.parameters).unwrap(),
        v["voice_parameters"]["native"]["parameters"]
    );
    for context in [json!(["richness", "richness"]), json!(["unknown"])] {
        v["voice_parameters"]["context_dimensions"] = context;
        assert!(request(&v).is_err());
    }
    v["voice_parameters"]["context_dimensions"] = json!(["richness", "rate_offset"]);
    request(&v).unwrap();
    v["voice_parameters"]["native"]["parameters"]["ri"]["value"] = 0.into();
    assert!(request(&v).is_err());
}

#[test]
fn limits_and_identity_are_checked_before_execution() {
    for value in [0, 5001] {
        let v = json!({"protocol_version":6,"request_id":1,"type":"engine_parameters_v1","engine_id":"dectalk","result":{"status":"busy","retry_after_ms":value}});
        assert!(response(&v).is_err());
    }
    for (pointer, bad) in [
        (
            "/voice_parameters/expected_identity/runtime_generation",
            json!(0),
        ),
        (
            "/voice_parameters/expected_identity/catalogue_revision",
            json!("A".repeat(64)),
        ),
        (
            "/voice_parameters/native/engine_id",
            json!("dectalk;command"),
        ),
        ("/request_id", json!(0)),
        ("/settings/rate", json!(2.1)),
    ] {
        let mut v = fixture("helper_synthesis");
        *v.pointer_mut(pointer).unwrap() = bad;
        assert!(request(&v).is_err(), "{pointer}");
    }
    let mut v = fixture("helper_synthesis");
    for n in 0..65 {
        v["voice_parameters"]["native"]["parameters"][format!("p{n}")] = json!({"op":"default"});
    }
    assert!(request(&v).is_err());
    let mut v = query();
    v["cursor"] = "page2".into();
    assert!(request(&v).is_err());
    v["expected_catalogue_revision"] = "1".repeat(64).into();
    request(&v).unwrap();
    v["cursor"] = "\n".into();
    assert!(request(&v).is_err());
    let mut v = catalogue();
    let p = v["result"]["parameters"][0].clone();
    v["result"]["parameters"] = (0..65)
        .map(|n| {
            let mut p = p.clone();
            p["id"] = format!("p{n}").into();
            p
        })
        .collect();
    assert!(response(&v).is_err());
}

#[test]
fn receipts_cannot_claim_unrequested_or_stale_application() {
    let req = request(&fixture("helper_synthesis")).unwrap();
    let good = fixture("helper_synthesis_started");
    for (pointer, bad) in [
        ("/request_id", json!(4)),
        ("/actual_voice_id", json!("other")),
        ("/native_application/identity/runtime_generation", json!(8)),
        (
            "/native_application/masked_parameters",
            json!(["not_requested"]),
        ),
        ("/native_application", Value::Null),
    ] {
        let mut v = good.clone();
        *v.pointer_mut(pointer).unwrap() = bad;
        assert!(
            response(&v)
                .and_then(|r| r.validate_for(&req, "dectalk"))
                .is_err(),
            "{pointer}"
        );
    }
    let mut v = good.clone();
    v["native_application"] = json!({"status":"common_only","plan_id":null,"identity":null,"masked_parameters":[],"reason":"unsupported_helper"});
    let r = response(&v).unwrap();
    assert!(r.validate_for(&req, "dectalk").is_err());
    let mut ordinary = fixture("helper_synthesis");
    ordinary["voice_parameters"]["unavailable_policy"] = "common_only".into();
    r.validate_for(&request(&ordinary).unwrap(), "dectalk")
        .unwrap();
    ordinary["voice_parameters"] = Value::Null;
    assert!(r
        .validate_for(&request(&ordinary).unwrap(), "dectalk")
        .is_err());
    v["native_application"] = Value::Null;
    response(&v)
        .unwrap()
        .validate_for(&request(&ordinary).unwrap(), "dectalk")
        .unwrap();
}

#[test]
fn explanation_distinguishes_prediction_from_applied_evidence() {
    let mut v = explanation("explain_response");
    v["result"]["parameters"][0]["read_back"] = true.into();
    assert!(response(&v).is_err());
    let mut v = explanation("applied_explanation");
    v["result"]["plan_id"] = Value::Null;
    assert!(response(&v).is_err());
    let mut v = explanation("applied_explanation");
    v["result"]["parameters"][0]["value"] = Value::Null;
    assert!(response(&v).is_err());
    v["result"]["parameters"][0]["read_back"] = false.into();
    response(&v).unwrap();
    let applied = json!({"protocol_version":6,"request_id":805,"type":"explain_voice_parameters_v1","source":{"mode":"applied","plan_id":"plan-23"}});
    response(&explanation("applied_explanation"))
        .unwrap()
        .validate_for(&request(&applied).unwrap(), "dectalk")
        .unwrap();
    let mut malformed = applied.clone();
    malformed["source"]["plan_id"] = "path/like-id".into();
    assert!(request(&malformed).is_err());
    let mut malformed_receipt = fixture("helper_synthesis_started");
    malformed_receipt["native_application"]["plan_id"] = "path/like-id".into();
    assert!(response(&malformed_receipt).is_err());
    let mut wrong = applied;
    wrong["source"]["plan_id"] = "other-plan".into();
    assert!(response(&explanation("applied_explanation"))
        .unwrap()
        .validate_for(&request(&wrong).unwrap(), "dectalk")
        .is_err());
}

fn split_pages() -> (CatalogueQuery, CatalogueResult, CatalogueResult) {
    let RequestBody::GetEngineParametersV1(q) = request(&query()).unwrap().body else {
        panic!()
    };
    let mut first = catalogue();
    let mut second = first.clone();
    first["result"]["parameters"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    first["result"]["next_cursor"] = "page:2".into();
    second["result"]["parameters"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let ResponseBody::EngineParametersV1 { result: a, .. } = response(&first).unwrap().body else {
        panic!()
    };
    let ResponseBody::EngineParametersV1 { result: b, .. } = response(&second).unwrap().body else {
        panic!()
    };
    (q, a, b)
}
#[test]
fn pages_are_joined_only_with_matching_runtime_and_mapping_identity() {
    let (q, a, b) = split_pages();
    let mut assembly = CatalogueAssembly::new(q.clone()).unwrap();
    assembly.push(&q, &a).unwrap();
    let next = assembly.next_query().unwrap().clone();
    assert_eq!(next.cursor.as_deref(), Some("page:2"));
    assert_eq!(
        next.expected_catalogue_revision.as_deref(),
        Some("1".repeat(64).as_str())
    );
    let mut wrong = b.clone();
    if let CatalogueResult::Ready { identity, .. } = &mut wrong {
        identity.runtime_generation += 1;
    }
    assert!(assembly.push(&next, &wrong).is_err());
    let mut wrong = b.clone();
    if let CatalogueResult::Ready { mappings, .. } = &mut wrong {
        mappings.clear();
    }
    assert!(assembly.push(&next, &wrong).is_err());
    assert!(assembly.push(&q, &b).is_err());
    assert_eq!(assembly.next_query(), Some(&next));
    assembly.push(&next, &b).unwrap();
    assert!(assembly.next_query().is_none());
    let full = assembly.finish().unwrap();
    assert_eq!(full.parameters.len(), 2);
    full.validate().unwrap();
}
#[test]
fn pagination_rejects_repetition_and_incomplete_references_atomically() {
    let (q, a, b) = split_pages();
    let mut assembly = CatalogueAssembly::new(q.clone()).unwrap();
    assembly.push(&q, &a).unwrap();
    let next = assembly.next_query().unwrap().clone();
    assert!(assembly.push(&next, &a).is_err()); // Duplicate ID and repeated cursor.
    let mut wrong = b.clone();
    if let CatalogueResult::Ready { parameters, .. } = &mut wrong {
        parameters[0].side_effects.push("missing".into());
    }
    assert!(assembly.push(&next, &wrong).is_err());
    assembly.push(&next, &b).unwrap();
    assert_eq!(assembly.finish().unwrap().parameters.len(), 2);
    assert!(CatalogueAssembly::new(q).unwrap().finish().is_err());
}

#[test]
fn catalogue_bounds_cover_encoded_bytes_and_complete_inventory() {
    let mut v = catalogue();
    let template = v["result"]["parameters"][0].clone();
    v["result"]["parameters"] = (0..64)
        .map(|n| {
            let mut p = template.clone();
            p["id"] = format!("p{n}").into();
            p["side_effects"] = (0..64)
                .map(|k| Value::String(format!("target{k:03}{}", "x".repeat(110))))
                .collect();
            p
        })
        .collect();
    assert!(encode_frame(&v).unwrap().len() > crate::control::MAX_CONTROL_PAYLOAD_BYTES);
    assert!(response(&v).is_err());
    let RequestBody::GetEngineParametersV1(q) = request(&query()).unwrap().body else {
        panic!()
    };
    let mut assembly = CatalogueAssembly::new(q).unwrap();
    for page in 0..8 {
        let mut v = catalogue();
        v["result"]["mappings"] = json!([]);
        v["result"]["next_cursor"] = format!("page{}", page + 1).into();
        v["result"]["parameters"] = (0..64)
            .map(|n| {
                let mut p = template.clone();
                p["id"] = format!("p{}", page * 64 + n).into();
                p
            })
            .collect();
        let ResponseBody::EngineParametersV1 { result, .. } = response(&v).unwrap().body else {
            panic!()
        };
        let q = assembly.next_query().unwrap().clone();
        assembly.push(&q, &result).unwrap();
    }
    let q = assembly.next_query().unwrap().clone();
    let mut v = catalogue();
    v["result"]["mappings"] = json!([]);
    v["result"]["parameters"] = json!([template]);
    let ResponseBody::EngineParametersV1 { mut result, .. } = response(&v).unwrap().body else {
        panic!()
    };
    assert!(assembly.push(&q, &result).is_err());
    // An empty final page can close a full inventory; rejection did not corrupt it.
    if let CatalogueResult::Ready { parameters, .. } = &mut result {
        parameters.clear();
    }
    assembly.push(&q, &result).unwrap();
    assert_eq!(assembly.finish().unwrap().parameters.len(), 512);
}

#[test]
fn busy_and_wrong_catalogue_replies_do_not_advance_pages() {
    let (q, a, b) = split_pages();
    let mut assembly = CatalogueAssembly::new(q.clone()).unwrap();
    assert!(assembly
        .push(&q, &CatalogueResult::Busy { retry_after_ms: 20 })
        .is_err());
    assert_eq!(assembly.next_query(), Some(&q));
    assembly.push(&q, &a).unwrap();
    let next = assembly.next_query().unwrap().clone();
    for mode in 0..3 {
        let mut bad = b.clone();
        if let CatalogueResult::Ready {
            identity, voice_id, ..
        } = &mut bad
        {
            match mode {
                0 => identity.catalogue_revision = "2".repeat(64),
                1 => identity.profile_id = "different.profile".into(),
                _ => *voice_id = Some("other".into()),
            }
        }
        assert!(assembly.push(&next, &bad).is_err());
        assert_eq!(assembly.next_query(), Some(&next));
    }
    assembly.push(&next, &b).unwrap();
    assembly.finish().unwrap();
    let req = request(&query()).unwrap();
    response(&catalogue())
        .unwrap()
        .validate_for(&req, "dectalk")
        .unwrap();
    assert!(response(&catalogue())
        .unwrap()
        .validate_for(&req, "eloquence")
        .is_err());
}

#[test]
fn response_shape_and_nested_null_fields_are_strict() {
    for pointer in [
        "",
        "/result",
        "/result/parameters/0",
        "/result/parameters/0/default",
        "/result/parameters/0/availability",
        "/result/mappings/0",
    ] {
        let mut v = catalogue();
        v.pointer_mut(pointer).unwrap()["unknown"] = Value::Null;
        assert!(response(&v).is_err(), "{pointer}");
    }
    for field in ["plan_id", "identity", "reason"] {
        let mut v = fixture("helper_synthesis_started");
        v["native_application"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(response(&v).is_err());
    }
    let raw = fixture("helper_synthesis_started").to_string();
    let bad = raw.replace(
        "\"plan_id\":\"plan-23\"",
        "\"plan_id\":\"other\",\"plan_id\":\"plan-23\"",
    );
    assert_ne!(raw, bad);
    assert!(serde_json::from_str::<Response>(&bad).is_err());
    for (pointer, value) in [
        ("/native_application/reason", json!("unexpected")),
        ("/native_application/plan_id", Value::Null),
        ("/native_application/identity", Value::Null),
    ] {
        let mut v = fixture("helper_synthesis_started");
        *v.pointer_mut(pointer).unwrap() = value;
        assert!(response(&v).is_err());
    }
}

#[test]
fn eloquence_runtime_exchanges_match_rust_codec_and_catalogue() {
    let captured: Value = serde_json::from_str(include_str!(
        "../../../../docs/protocol-fixtures/eloquence-helper6-runtime.json"
    ))
    .unwrap();
    for exchange in captured["exchanges"].as_array().unwrap() {
        let sent = request(&exchange["request"]).unwrap();
        let received = response(&exchange["response"]).unwrap();
        received.validate_for(&sent, "eloquence").unwrap();
        if let (
            RequestBody::GetEngineParametersV1(query),
            ResponseBody::EngineParametersV1 {
                result: page @ CatalogueResult::Ready { .. },
                ..
            },
        ) = (&sent.body, &received.body)
        {
            let mut assembly = CatalogueAssembly::new(query.clone()).unwrap();
            assembly.push(query, page).unwrap();
            let catalogue = assembly.finish().unwrap();
            assert_eq!(catalogue.parameters.len(), 8);
            assert_eq!(catalogue.identity.schema_id, "eloquence.eci-units.v1");
        }
    }
}
