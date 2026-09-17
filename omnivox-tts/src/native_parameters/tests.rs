use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;
use crate::contracts::{PhysicalVoiceId, VoiceSelector};
use crate::voice_choices::{Adjustment, VoiceChoice, VoiceStylePatch};

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()
}

fn catalogue_json() -> Value {
    let mut result = fixtures()["messages"]["catalogue_response"]["result"].clone();
    let object = result.as_object_mut().unwrap();
    object.remove("status");
    object.remove("next_cursor");
    object.insert("engine_id".into(), json!("dectalk"));
    result
}

fn catalogue() -> ParameterCatalogue {
    ParameterCatalogue::from_json(&serde_json::to_vec(&catalogue_json()).unwrap()).unwrap()
}

fn patch(parameters: Value) -> NativePatch {
    NativePatch::from_json(
        &serde_json::to_vec(&json!({
            "engine_id":"dectalk", "schema_id":"dectalk.design-voice.v1", "parameters": parameters
        }))
        .unwrap(),
    )
    .unwrap()
}

fn mapped() -> BTreeMap<String, NativeValue> {
    BTreeMap::from([
        ("ri".into(), NativeValue::Integer(63)),
        ("sm".into(), NativeValue::Integer(12)),
    ])
}

fn plan(
    c: &ParameterCatalogue,
    values: &BTreeMap<String, NativeValue>,
    p: Option<&NativePatch>,
    context: &VoiceStylePatch,
) -> Result<NativePlan, ParameterError> {
    compose(
        c,
        &c.identity,
        &PhysicalVoiceId::new(&c.engine_id, c.voice_id.as_ref().unwrap()),
        values,
        p,
        context,
    )
}

#[test]
fn catalogue_and_native_records_round_trip_the_independent_examples() {
    let c = catalogue();
    assert_eq!(serde_json::to_value(&c).unwrap(), catalogue_json());
    let fixtures = fixtures();
    for choice in fixtures["messages"]["registration"]["definitions"][0]["definition"]["choices"]
        .as_array()
        .unwrap()
    {
        if choice["native"].is_null() {
            continue;
        }
        let p = NativePatch::from_json(&serde_json::to_vec(&choice["native"]).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(p).unwrap(), choice["native"]);
    }
}

#[test]
fn independent_composition_cases_keep_the_actual_engine_mapping() {
    for case in fixtures()["composition_cases"].as_array().unwrap() {
        let mut c = catalogue();
        let eci = case["engine_id"] == "eloquence";
        if eci {
            c.engine_id = "eloquence".into();
            c.voice_id = Some("v1".into());
            c.identity.schema_id = "eloquence.eci-units.v1".into();
            c.parameters[0].id = "breathiness".into();
            c.parameters[0].default.value = Some(NativeValue::Integer(0));
            c.parameters[1].id = "volume".into();
            c.parameters[1].default.value = Some(NativeValue::Integer(92));
            c.mappings[0].native_outputs = vec!["breathiness".into(), "volume".into()];
            c.mappings[0].common_inputs.push(CommonInput::Volume);
        }
        let native = NativePatch::from_json(
            &serde_json::to_vec(&json!({
                "engine_id":c.engine_id, "schema_id":c.identity.schema_id,
                "parameters":case["native_parameters"]
            }))
            .unwrap(),
        )
        .unwrap();
        let context: VoiceStylePatch = serde_json::from_value(case["context"].clone()).unwrap();
        let values: BTreeMap<String, NativeValue> =
            serde_json::from_value(case["mapped_common"].clone()).unwrap();
        let result = plan(&c, &values, Some(&native), &context).unwrap();
        for (id, expected) in case["expected_native_subset"].as_object().unwrap() {
            assert_eq!(
                serde_json::to_value(&result.parameters[id].value).unwrap(),
                *expected,
                "{} {id}",
                case["name"]
            );
        }
        let masked: Vec<_> = result
            .parameters
            .iter()
            .filter(|(_, p)| p.masked_native)
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(
            serde_json::to_value(masked).unwrap(),
            case["expected_masked"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn no_native_patch_preserves_common_outputs_exactly() {
    let c = catalogue();
    let values = mapped();
    let result = plan(&c, &values, None, &VoiceStylePatch::default()).unwrap();
    for (id, value) in values {
        assert_eq!(result.parameters[&id].value, Some(value));
    }
    assert!(result
        .parameters
        .values()
        .all(|p| p.origin == ValueOrigin::CommonMapping));
}

#[test]
fn default_omission_and_zero_have_distinct_provenance() {
    let c = catalogue();
    let inherited = plan(&c, &mapped(), None, &VoiceStylePatch::default()).unwrap();
    let reset = plan(
        &c,
        &mapped(),
        Some(&patch(json!({"sm":{"op":"default"}}))),
        &VoiceStylePatch::default(),
    )
    .unwrap();
    let zero = plan(
        &c,
        &mapped(),
        Some(&patch(json!({"sm":{"op":"set","value":0}}))),
        &VoiceStylePatch::default(),
    )
    .unwrap();
    assert_eq!(
        inherited.parameters["sm"].value,
        Some(NativeValue::Integer(12))
    );
    assert_eq!(reset.parameters["sm"].value, Some(NativeValue::Integer(3)));
    assert_eq!(reset.parameters["sm"].origin, ValueOrigin::NativeDefault);
    assert_eq!(zero.parameters["sm"].value, Some(NativeValue::Integer(0)));
    assert_eq!(zero.parameters["sm"].origin, ValueOrigin::NativeSet);
}

#[test]
fn explicit_default_context_masks_choice_even_without_mapped_output() {
    let c = catalogue();
    let context: VoiceStylePatch =
        serde_json::from_value(json!({"richness":{"op":"default"}})).unwrap();
    let result = plan(
        &c,
        &BTreeMap::new(),
        Some(&patch(json!({"sm":{"op":"set","value":55}}))),
        &context,
    )
    .unwrap();
    assert_eq!(result.parameters["sm"].value, Some(NativeValue::Integer(3)));
    assert_eq!(result.parameters["sm"].origin, ValueOrigin::ContextMapping);
    assert!(result.parameters["sm"].masked_native);
}

#[test]
fn output_effects_do_not_mask_native_voice_controls() {
    let c = catalogue();
    let context: VoiceStylePatch =
        serde_json::from_value(json!({"gain":{"op":"set","value":0.1}})).unwrap();
    let result = plan(
        &c,
        &mapped(),
        Some(&patch(json!({"sm":{"op":"set","value":55}}))),
        &context,
    )
    .unwrap();
    assert_eq!(result.parameters["sm"].origin, ValueOrigin::NativeSet);
    assert!(!result.parameters["sm"].masked_native);
}

#[test]
fn explicit_zero_rate_offset_still_masks_a_pinned_native_speed() {
    let mut c = catalogue();
    c.parameters[0].id = "speed".into();
    c.mappings = vec![CommonMapping {
        common_inputs: vec![CommonInput::Rate, CommonInput::RateOffset],
        native_outputs: vec!["speed".into()],
    }];
    let p = patch(json!({"speed":{"op":"set","value":80}}));
    let values = BTreeMap::from([("speed".into(), NativeValue::Integer(50))]);
    let inherited = plan(&c, &values, Some(&p), &VoiceStylePatch::default()).unwrap();
    let context = serde_json::from_value(json!({"rate_offset":{"op":"set","value":0}})).unwrap();
    let explicit = plan(&c, &values, Some(&p), &context).unwrap();
    assert_eq!(
        inherited.parameters["speed"].value,
        Some(NativeValue::Integer(80))
    );
    assert_eq!(
        explicit.parameters["speed"].value,
        Some(NativeValue::Integer(50))
    );
    assert!(explicit.parameters["speed"].masked_native);
}

#[test]
fn unavailable_or_read_only_controls_reject_the_whole_native_block() {
    let p = patch(json!({"sm":{"op":"set","value":55}}));
    let mut c = catalogue();
    c.parameters[1].adjustable = false;
    assert!(plan(&c, &mapped(), Some(&p), &VoiceStylePatch::default()).is_err());
    for status in [
        AvailabilityStatus::VoiceUnavailable,
        AvailabilityStatus::RuntimeUnsupported,
        AvailabilityStatus::NotChecked,
    ] {
        let mut c = catalogue();
        c.parameters[1].availability = ParameterAvailability {
            status,
            reason: Some("Unavailable on this runtime".into()),
        };
        assert!(c.validate().is_ok());
        assert!(plan(&c, &BTreeMap::new(), Some(&p), &VoiceStylePatch::default()).is_err());
    }
}

#[test]
fn repeated_same_voice_choices_never_reuse_mutable_plan_state() {
    let c = catalogue();
    let before = c.clone();
    let values = mapped();
    for value in [55, 80, 55] {
        let p = patch(json!({"sm":{"op":"set","value":value}}));
        let result = plan(&c, &values, Some(&p), &VoiceStylePatch::default()).unwrap();
        assert_eq!(
            result.parameters["sm"].value,
            Some(NativeValue::Integer(value))
        );
    }
    assert_eq!(c, before);
    assert_eq!(values, mapped());
    assert_eq!(
        plan(&c, &values, None, &VoiceStylePatch::default())
            .unwrap()
            .parameters["sm"]
            .value,
        Some(NativeValue::Integer(12))
    );
}

#[test]
fn native_block_is_all_or_nothing_even_when_context_masks_invalid_value() {
    let c = catalogue();
    let before = c.clone();
    let p = patch(json!({"sm":{"op":"set","value":55}, "ri":{"op":"set","value":101}}));
    let context: VoiceStylePatch =
        serde_json::from_value(json!({"richness":{"op":"set","value":1.0}})).unwrap();
    assert!(plan(&c, &mapped(), Some(&p), &context).is_err());
    assert_eq!(c, before);
}

#[test]
fn dependencies_reassert_contextual_winners_and_reject_cycles() {
    let mut c = catalogue();
    c.parameters[0].side_effects = vec!["sm".into()];
    let context: VoiceStylePatch =
        serde_json::from_value(json!({"richness":{"op":"set","value":0.5}})).unwrap();
    let result = plan(
        &c,
        &mapped(),
        Some(&patch(json!({"sm":{"op":"set","value":55}}))),
        &context,
    )
    .unwrap();
    assert_eq!(result.application_order, ["ri", "sm"]);
    assert_eq!(
        result.parameters["sm"].value,
        Some(NativeValue::Integer(12))
    );
    assert_eq!(result.parameters["sm"].origin, ValueOrigin::ContextMapping);
    c.parameters[1].side_effects = vec!["ri".into()];
    assert_eq!(
        plan(&c, &mapped(), None, &context),
        Err(ParameterError::DependencyCycle)
    );
}

#[test]
fn unknown_numeric_default_is_honest_and_cannot_restore_a_side_effect() {
    let mut c = catalogue();
    c.parameters[1].default.source = DefaultSource::Unknown;
    c.parameters[1].default.value = None;
    let p = patch(json!({"sm":{"op":"default"}}));
    let result = plan(&c, &mapped(), Some(&p), &VoiceStylePatch::default()).unwrap();
    assert_eq!(result.parameters["sm"].value, None);
    c.parameters[0].side_effects = vec!["sm".into()];
    assert!(plan(&c, &mapped(), Some(&p), &VoiceStylePatch::default()).is_err());
    c.parameters[1].default.reset_supported = false;
    assert!(c.validate().is_err());
}

#[test]
fn evidence_is_bound_to_runtime_and_physical_voice_not_just_engine_name() {
    let c = catalogue();
    let mut stale = c.identity.clone();
    stale.runtime_generation += 1;
    let actual = PhysicalVoiceId::new("dectalk", "paul");
    assert_eq!(
        compose(
            &c,
            &stale,
            &actual,
            &mapped(),
            None,
            &VoiceStylePatch::default()
        ),
        Err(ParameterError::StaleIdentity)
    );
    assert_eq!(
        compose(
            &c,
            &c.identity,
            &PhysicalVoiceId::new("dectalk", "betty"),
            &mapped(),
            None,
            &VoiceStylePatch::default()
        ),
        Err(ParameterError::IdentityMismatch)
    );
    let p = patch(json!({"sm":{"op":"set","value":55}}));
    assert!(p
        .validate_for(
            &c,
            &VoiceSelector::Properties {
                engine_id: None,
                language: None,
                gender: None
            }
        )
        .is_err());
    assert!(p
        .validate_for(
            &c,
            &VoiceSelector::Exact(PhysicalVoiceId::new("dectalk", "betty"))
        )
        .is_err());
}

#[test]
fn unknown_schema_is_inert_preservable_data_but_not_executable() {
    let c = catalogue();
    let mut p = patch(json!({"future":{"op":"set","value":false}}));
    p.schema_id = "future.v1".into();
    let saved = serde_json::to_vec(&p).unwrap();
    assert_eq!(NativePatch::from_json(&saved).unwrap(), p);
    assert!(plan(&c, &mapped(), Some(&p), &VoiceStylePatch::default()).is_err());
}

#[test]
fn native_fixture_rejections_include_duplicate_keys_and_vendor_commands() {
    let c = catalogue();
    let selector = VoiceSelector::Exact(PhysicalVoiceId::new("dectalk", "paul"));
    for case in fixtures()["invalid_cases"].as_array().unwrap() {
        let data = if let Some(raw) = case["raw_parameters"].as_str() {
            format!("{{\"engine_id\":\"dectalk\",\"schema_id\":\"dectalk.design-voice.v1\",\"parameters\":{raw}}}").into_bytes()
        } else {
            serde_json::to_vec(&json!({"engine_id":"dectalk","schema_id":"dectalk.design-voice.v1","parameters":case["parameters"]})).unwrap()
        };
        assert!(
            NativePatch::from_json(&data)
                .and_then(|p| p.validate_for(&c, &selector))
                .is_err(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn scalars_preserve_false_and_zero_and_do_not_force_values_onto_ui_steps() {
    for case in fixtures()["generic_value_cases"].as_array().unwrap() {
        let kind: ValueType = serde_json::from_value(case["type"].clone()).unwrap();
        let Adjustment::Set { value } =
            serde_json::from_value::<Adjustment<NativeValue>>(case["operation"].clone()).unwrap()
        else {
            panic!()
        };
        assert!(kind.accepts(&value));
    }
    let kind = ValueType::Number {
        minimum: 0.0,
        maximum: 1.0,
        step: 0.1,
    };
    assert!(kind.accepts(&NativeValue::Number(0.15)));
    assert!(kind.accepts(&NativeValue::Integer(0)));
    assert!(!kind.accepts(&NativeValue::Number(f64::NAN)));
    assert!(!kind.accepts(&NativeValue::Number(f64::INFINITY)));
    assert!(!kind.accepts(&NativeValue::Boolean(false)));
    assert!(!ValueType::Integer {
        minimum: 0,
        maximum: 100,
        step: 1
    }
    .accepts(&NativeValue::Number(1.0)));
}

#[test]
fn catalogue_checks_duplicates_references_scopes_ranges_and_required_nullables() {
    let base = catalogue_json();
    let mut variants = Vec::new();
    let mut v = base.clone();
    v["parameters"][1]["id"] = json!("ri");
    variants.push(v);
    let mut v = base.clone();
    v["parameters"][0]["default"]["value"] = json!(101);
    variants.push(v);
    let mut v = base.clone();
    v["parameters"][0]["value_type"]["step"] = json!(0);
    variants.push(v);
    let mut v = base.clone();
    v["parameters"][0]["side_effects"] = json!(["missing"]);
    variants.push(v);
    let mut v = base.clone();
    v["mappings"][0]["native_outputs"] = json!(["missing"]);
    variants.push(v);
    let mut v = base.clone();
    v["parameters"][0].as_object_mut().unwrap().remove("unit");
    variants.push(v);
    let mut v = base.clone();
    v["parameters"][0]["default"]
        .as_object_mut()
        .unwrap()
        .remove("value");
    variants.push(v);
    let mut v = base.clone();
    v["voice_id"] = Value::Null;
    variants.push(v);
    for value in variants {
        assert!(ParameterCatalogue::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    for scope in [ParameterScope::Engine, ParameterScope::Startup] {
        let mut c = catalogue();
        c.parameters[1].scope = scope;
        assert!(plan(
            &c,
            &mapped(),
            Some(&patch(json!({"sm":{"op":"set","value":55}}))),
            &VoiceStylePatch::default()
        )
        .is_err());
    }
    let mut c = catalogue();
    c.voice_id = Some("en+f1".into());
    assert!(c.validate().is_ok());
}

#[test]
fn payload_and_collection_bounds_are_enforced() {
    assert!(
        NativePatch::from_json(&vec![b' '; crate::control::MAX_CONTROL_PAYLOAD_BYTES + 1]).is_err()
    );
    let mut p = patch(json!({}));
    for i in 0..=MAX_NATIVE_OPERATIONS {
        p.parameters.insert(format!("p{i}"), Adjustment::Default {});
    }
    assert!(p.validate_shape().is_err());
    let mut c = catalogue();
    c.parameters = vec![c.parameters[0].clone(); MAX_PARAMETERS + 1];
    assert!(c.validate().is_err());
    assert!(ValueType::Enum {
        choices: vec![
            EnumChoice {
                value: "x".into(),
                label: "X".into()
            };
            MAX_ENUM_CHOICES + 1
        ]
    }
    .validate()
    .is_err());
}

#[test]
fn old_choice_grammar_does_not_silently_ignore_native_parameters() {
    let value = fixtures()["messages"]["registration"]["definitions"][0]["definition"]["choices"]
        [0]
    .clone();
    assert!(serde_json::from_value::<VoiceChoice>(value).is_err());
}
