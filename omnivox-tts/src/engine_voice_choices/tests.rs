use super::*;
use crate::contracts::{Availability, EngineHealth, VoiceDescriptor};
use crate::logical_voices::LogicalVoiceRegistry;
use crate::native_parameters::{CommonInput, NativeValue};
use crate::resolver::ResolutionReason;
use crate::{VoiceInfo, VoiceQuality};
use serde_json::{json, Value};

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap()
}
fn body() -> Value {
    let mut value = fixtures()["messages"]["registration"].clone();
    for field in ["protocol_version", "request_id", "type"] {
        value.as_object_mut().unwrap().remove(field);
    }
    value
}
fn request(value: &Value) -> VoiceRegistrationV3 {
    VoiceRegistrationV3::from_json(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn definition() -> EngineLayeredVoiceDefinition {
    let EngineRegisteredVoiceDefinition::EngineLayered(v) = request(&body()).definitions.remove(0)
    else {
        panic!()
    };
    v
}
fn catalogues() -> Vec<ParameterCatalogue> {
    let mut value = fixtures()["messages"]["catalogue_response"]["result"].clone();
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
fn inventory() -> Vec<EngineDescriptor> {
    [("dectalk", "paul"), ("eloquence", "v1"), ("espeak", "en")]
        .into_iter()
        .map(|(id, voice)| {
            let mut d = EngineDescriptor::unavailable(id, "unused");
            d.availability = Availability::Available;
            d.health = EngineHealth::Healthy;
            d.voices.push(VoiceDescriptor::from_voice_info(
                id,
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
fn snapshot<'a>(
    inventory: &'a [EngineDescriptor],
    catalogues: &'a [ParameterCatalogue],
) -> NativeCatalogueSnapshot<'a> {
    NativeCatalogueSnapshot::new(
        inventory,
        &catalogues
            .iter()
            .map(ParameterKnowledge::Ready)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
fn resolution(index: usize) -> VoiceResolution {
    VoiceResolution {
        logical_voice_id: "bolden".into(),
        requested: None,
        realized: PhysicalVoiceId::new("dectalk", "paul"),
        reason: if index == 0 {
            ResolutionReason::Preferred
        } else {
            ResolutionReason::ExplicitAlternative {
                preference_index: index,
            }
        },
        failed_attempts: vec![],
    }
}
fn prepared_parameters(result: PreparedEngineVoiceStyle) -> VoiceParameters {
    let NativeChoiceExecution::Parameters(p) = result.native else {
        panic!("expected native parameters")
    };
    *p
}

#[test]
fn independent_native_choice_fixture_registers_without_losing_occurrence_identity() {
    let value = body();
    let request = request(&value);
    assert_eq!(serde_json::to_value(&request).unwrap(), value);
    let (inventory, catalogues) = (inventory(), catalogues());
    let snapshot = snapshot(&inventory, &catalogues);
    let mut registry = LogicalVoiceRegistry::default();
    let reply = registry.register_v3(request.clone(), &snapshot).unwrap();
    assert!(reply.unresolved_logical_voice_ids.is_empty());
    assert_eq!(
        serde_json::to_value(reply.native_status.clone()).unwrap(),
        fixtures()["messages"]["registration_ack"]["native_status"]
    );
    assert!(registry.is_engine_layered("bolden"));
    let RegisteredVoiceDefinition::EngineLayered(saved) = &registry.registered_definitions()[0]
    else {
        panic!()
    };
    assert_eq!(saved, &definition());
    assert_eq!(registry.register_v3(request, &snapshot).unwrap(), reply);
}

#[test]
fn invalid_native_replacement_and_conflicting_retries_leave_all_registry_state_intact() {
    let (inventory, catalogues) = (inventory(), catalogues());
    let snapshot = snapshot(&inventory, &catalogues);
    let mut registry = LogicalVoiceRegistry::default();
    registry.register_v3(request(&body()), &snapshot).unwrap();
    let saved = registry.clone();
    let mut invalid = body();
    invalid["registry_generation"] = json!(42);
    invalid["definitions"][0]["definition"]["choices"][1]["native"]["parameters"]["sm"]["value"] =
        json!(101);
    assert!(registry.register_v3(request(&invalid), &snapshot).is_err());
    let mut conflict = body();
    conflict["definitions"][0]["definition"]["choices"][0]["native"]["parameters"]["sm"]["value"] =
        json!(56);
    assert!(registry.register_v3(request(&conflict), &snapshot).is_err());
    let mut stale = body();
    stale["registry_generation"] = json!(40);
    assert!(registry.register_v3(request(&stale), &snapshot).is_err());
    assert_eq!(registry.generation(), saved.generation());
    assert_eq!(
        registry.registered_definitions(),
        saved.registered_definitions()
    );
    assert_eq!(registry.definitions(), saved.definitions());
    assert_eq!(registry.bindings(), saved.bindings());
    assert_eq!(registry.fallback_policy(), saved.fallback_policy());
    // The old API may replace at a new generation, but cannot erase native data on a retry.
    let legacy = vec![RegisteredVoiceDefinition::Layered(
        definition().common_projection(),
    )];
    assert!(registry
        .register_v2(41, legacy.clone(), Default::default(), &inventory)
        .is_err());
    registry
        .register_v2(42, legacy, Default::default(), &inventory)
        .unwrap();
    assert!(!registry.is_engine_layered("bolden"));
    assert!(saved.is_engine_layered("bolden"));
}

#[test]
fn missing_engines_unknown_schemas_and_busy_metadata_remain_saved_but_inert() {
    let inventory = inventory();
    let catalogues = catalogues();
    let mut registry = LogicalVoiceRegistry::default();
    let missing = snapshot(&[], &[]);
    let reply = registry.register_v3(request(&body()), &missing).unwrap();
    assert!(reply
        .native_status
        .iter()
        .all(|s| s.status == NativeSupport::Unavailable && s.reason.is_some()));
    let busy = NativeCatalogueSnapshot::new(
        &inventory,
        &[
            ParameterKnowledge::Deferred {
                engine_id: "dectalk",
            },
            ParameterKnowledge::Unavailable {
                engine_id: "eloquence",
            },
        ],
    )
    .unwrap();
    let reply = registry.register_v3(request(&body()), &busy).unwrap();
    assert_eq!(reply.native_status[0].status, NativeSupport::Deferred);
    assert_eq!(reply.native_status[2].status, NativeSupport::Unavailable);
    let mut unknown = body();
    unknown["registry_generation"] = json!(42);
    unknown["definitions"][0]["definition"]["choices"][0]["native"] = json!({"engine_id":"dectalk","schema_id":"future.v1","parameters":{"future":{"op":"set","value":false}}});
    let original = request(&unknown);
    let reply = registry
        .register_v3(original.clone(), &snapshot(&inventory, &catalogues))
        .unwrap();
    assert_eq!(reply.native_status[0].status, NativeSupport::Unavailable);
    let RegisteredVoiceDefinition::EngineLayered(saved) = &registry.registered_definitions()[0]
    else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(&saved.choices[0].native).unwrap(),
        unknown["definitions"][0]["definition"]["choices"][0]["native"]
    );
}

#[test]
fn native_wire_body_is_bounded_strict_and_old_modes_do_not_accept_extensions() {
    let value = body();
    let choices = "/definitions/0/definition/choices/0";
    for edit in [0, 1, 2, 3, 4, 5] {
        let mut bad = value.clone();
        let choice = bad.pointer_mut(choices).unwrap();
        match edit {
            0 => {
                choice.as_object_mut().unwrap().remove("native");
            }
            1 => choice["native"]["parameters"]["sm"] = json!({"op":"default","value":0}),
            2 => choice["native"]["parameters"]["sm"] = json!({"op":"add","value":0}),
            3 => choice["native"]["engine_id"] = json!("eloquence"),
            4 => {
                choice["selector"] =
                    json!({"kind":"properties","engine_id":null,"language":"en-US","gender":null})
            }
            _ => choice["unknown"] = json!(true),
        }
        assert!(
            VoiceRegistrationV3::from_json(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "case {edit}"
        );
    }
    let encoded =
        value
            .to_string()
            .replacen("\"sm\":", "\"sm\":{\"op\":\"set\",\"value\":0},\"sm\":", 1);
    assert!(VoiceRegistrationV3::from_json(encoded.as_bytes()).is_err());
    let mut too_large = value.to_string();
    too_large.extend(std::iter::repeat_n(
        ' ',
        crate::control::MAX_CONTROL_PAYLOAD_BYTES,
    ));
    assert!(VoiceRegistrationV3::from_json(too_large.as_bytes()).is_err());
    assert!(
        serde_json::from_value::<RegisteredVoiceDefinition>(value["definitions"][0].clone())
            .is_err()
    );
    let mut old_layered = value.clone();
    old_layered["definitions"][0]["mode"] = json!("layered");
    assert!(VoiceRegistrationV3::from_json(&serde_json::to_vec(&old_layered).unwrap()).is_err());
    let mut legacy = json!({"mode":"legacy","definition":{"id":"plain","language":null,"preferences":[],"acss":{}}});
    assert!(serde_json::from_value::<EngineRegisteredVoiceDefinition>(legacy.clone()).is_ok());
    let mut overflow = value.clone();
    overflow["definitions"] = json!([legacy.clone()]);
    overflow["definitions"][0]["definition"]["acss"]["rate"] = json!(1e39);
    assert!(VoiceRegistrationV3::from_json(&serde_json::to_vec(&overflow).unwrap()).is_err());
    let EngineRegisteredVoiceDefinition::Legacy(mut typed) =
        serde_json::from_value(legacy.clone()).unwrap()
    else {
        panic!()
    };
    let mut registry = LogicalVoiceRegistry::default();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        typed.effects.gain = Some(value);
        let mut request = request(&body());
        request.definitions = vec![EngineRegisteredVoiceDefinition::Legacy(typed.clone())];
        assert!(registry.register_v3(request, &snapshot(&[], &[])).is_err());
        assert_eq!(registry.generation(), 0);
    }
    legacy["definition"]["native"] = Value::Null;
    assert!(serde_json::from_value::<EngineRegisteredVoiceDefinition>(legacy).is_err());
}

#[test]
fn selected_occurrence_preserves_native_zero_default_and_equal_context() {
    let mut definition = definition();
    definition.choices[0].native.as_mut().unwrap().parameters =
        serde_json::from_value(json!({"sm":{"op":"set","value":0},"ri":{"op":"default"}})).unwrap();
    let context: VoiceStylePatch = serde_json::from_value(json!({"richness":{"op":"set","value":0.5},"rate_offset":{"op":"set","value":0},"gain":{"op":"default"}})).unwrap();
    let (inventory, catalogues) = (inventory(), catalogues());
    let snapshot = snapshot(&inventory, &catalogues);
    let first = definition
        .prepare(
            &resolution(0),
            &context,
            1.8,
            Some(0.2),
            &snapshot,
            UnavailablePolicy::Require,
        )
        .unwrap();
    assert_eq!(first.common.choice_id.as_deref(), Some("paul-main"));
    assert_eq!(first.common.acss.richness, Some(0.5));
    assert_eq!(first.common.acss.rate, None); // Zero offset retains the host rate.
    assert_eq!(first.common.effects.pan, Some(0.2));
    let first = prepared_parameters(first);
    assert_eq!(
        first.native.parameters["sm"],
        Adjustment::Set {
            value: NativeValue::Integer(0)
        }
    );
    assert_eq!(first.native.parameters["ri"], Adjustment::Default {});
    assert_eq!(
        first.context_dimensions,
        vec![
            CommonInput::RateOffset,
            CommonInput::Richness,
            CommonInput::Gain
        ]
    );
    first.validate().unwrap();
    let second = prepared_parameters(
        definition
            .prepare(
                &resolution(1),
                &VoiceStylePatch::default(),
                0.5,
                None,
                &snapshot,
                UnavailablePolicy::Require,
            )
            .unwrap(),
    );
    assert_eq!(
        second.native.parameters["sm"],
        Adjustment::Set {
            value: NativeValue::Integer(80)
        }
    );
    assert!(second.context_dimensions.is_empty());
    assert_eq!(
        first.native.parameters["sm"],
        Adjustment::Set {
            value: NativeValue::Integer(0)
        }
    );
    let mut fallback = resolution(0);
    fallback.reason = ResolutionReason::FallbackEngine { fallback_index: 0 };
    fallback.realized = PhysicalVoiceId::new("espeak", "en");
    let fallback = definition
        .prepare(
            &fallback,
            &VoiceStylePatch::default(),
            0.5,
            None,
            &snapshot,
            UnavailablePolicy::Require,
        )
        .unwrap();
    assert_eq!(fallback.native, NativeChoiceExecution::NotRequested);
    assert_eq!(fallback.common.choice_id, None);
    assert_eq!(fallback.common.acss.richness, Some(0.5));
}

#[test]
fn stale_or_inapplicable_entire_blocks_degrade_only_with_explicit_policy() {
    let definition = definition();
    let inventory = inventory();
    let missing = snapshot(&inventory, &[]);
    assert!(definition
        .prepare(
            &resolution(0),
            &VoiceStylePatch::default(),
            0.5,
            None,
            &missing,
            UnavailablePolicy::Require
        )
        .is_err());
    let mut changed = catalogues();
    changed[0].parameters[1].value_type = crate::native_parameters::ValueType::Integer {
        minimum: 0,
        maximum: 50,
        step: 1,
    };
    let changed = snapshot(&inventory, &changed);
    for snapshot in [&missing, &changed] {
        let result = definition
            .prepare(
                &resolution(0),
                &VoiceStylePatch::default(),
                0.5,
                None,
                snapshot,
                UnavailablePolicy::CommonOnly,
            )
            .unwrap();
        let NativeChoiceExecution::CommonOnly { reason } = result.native else {
            panic!()
        };
        assert!(!reason.is_empty());
        assert!(definition
            .prepare(
                &resolution(0),
                &VoiceStylePatch::default(),
                0.5,
                None,
                snapshot,
                UnavailablePolicy::Require
            )
            .is_err());
    }
    let mut wrong_voice = resolution(0);
    wrong_voice.realized = PhysicalVoiceId::new("dectalk", "betty");
    assert!(definition
        .prepare(
            &wrong_voice,
            &VoiceStylePatch::default(),
            0.5,
            None,
            &missing,
            UnavailablePolicy::CommonOnly
        )
        .is_err());
}

#[test]
fn metadata_refresh_changes_execution_identity_without_changing_saved_registry_generation() {
    let (inventory, catalogues) = (inventory(), catalogues());
    let current = snapshot(&inventory, &catalogues);
    let mut registry = LogicalVoiceRegistry::default();
    registry.register_v3(request(&body()), &current).unwrap();
    let original = registry.registered_definitions().to_vec();
    let old = prepared_parameters(
        definition()
            .prepare(
                &resolution(0),
                &VoiceStylePatch::default(),
                0.5,
                None,
                &current,
                UnavailablePolicy::Require,
            )
            .unwrap(),
    );
    let mut refreshed = catalogues.clone();
    for c in &mut refreshed {
        c.identity.runtime_generation += 1;
    }
    let refreshed = snapshot(&inventory, &refreshed);
    registry.register_v3(request(&body()), &refreshed).unwrap();
    let new = prepared_parameters(
        definition()
            .prepare(
                &resolution(0),
                &VoiceStylePatch::default(),
                0.5,
                None,
                &refreshed,
                UnavailablePolicy::Require,
            )
            .unwrap(),
    );
    assert_eq!(registry.generation(), 41);
    assert_eq!(registry.registered_definitions(), original);
    assert_ne!(old.expected_identity, new.expected_identity);
    assert_eq!(old.expected_identity, catalogues[0].identity);
}

#[test]
fn conflicting_metadata_and_unsupported_voice_parameters_are_not_mistaken_for_ready() {
    let (mut inventory, mut catalogues) = (inventory(), catalogues());
    assert!(NativeCatalogueSnapshot::new(
        &inventory,
        &[
            ParameterKnowledge::Ready(&catalogues[0]),
            ParameterKnowledge::Ready(&catalogues[0])
        ]
    )
    .is_err());
    let mut stale = catalogues[0].clone();
    stale.voice_id = Some("betty".into());
    stale.identity.runtime_generation += 1;
    assert!(NativeCatalogueSnapshot::new(
        &inventory,
        &[
            ParameterKnowledge::Ready(&catalogues[0]),
            ParameterKnowledge::Ready(&stale)
        ]
    )
    .is_err());
    assert!(NativeCatalogueSnapshot::new(
        &inventory,
        &[
            ParameterKnowledge::Ready(&catalogues[0]),
            ParameterKnowledge::Unavailable {
                engine_id: "dectalk"
            }
        ]
    )
    .is_err());
    let mut registry = LogicalVoiceRegistry::default();
    catalogues[0].parameters[1].availability.status = AvailabilityStatus::NotChecked;
    catalogues[0].parameters[1].availability.reason =
        Some("Voice applicability has not been checked".into());
    let reply = registry
        .register_v3(request(&body()), &snapshot(&inventory, &catalogues))
        .unwrap();
    assert_eq!(reply.native_status[0].status, NativeSupport::Deferred);
    inventory[0].voices[0].availability = Availability::Unavailable {
        reason: "disabled".into(),
    };
    let reply = registry
        .register_v3(request(&body()), &snapshot(&inventory, &catalogues))
        .unwrap();
    assert_eq!(reply.native_status[0].status, NativeSupport::Unavailable);
}

#[test]
fn oversized_native_acknowledgement_is_rejected_before_publication() {
    let mut request = request(&body());
    let mut template = definition();
    template.choices.truncate(1);
    template.choices[0]
        .native
        .as_mut()
        .unwrap()
        .parameters
        .clear();
    template.choices = (0..32)
        .map(|i| {
            let mut c = template.choices[0].clone();
            c.id = format!("c{i}");
            c
        })
        .collect();
    request.definitions.clear();
    for i in 0..256 {
        let mut voice = template.clone();
        voice.id = format!("voice{i}-{}", "x".repeat(110));
        request
            .definitions
            .push(EngineRegisteredVoiceDefinition::EngineLayered(voice));
        if serde_json::to_vec(&request).unwrap().len() > crate::control::MAX_CONTROL_PAYLOAD_BYTES {
            request.definitions.pop();
            break;
        }
    }
    request.validate().unwrap();
    let mut registry = LogicalVoiceRegistry::default();
    let error = registry
        .register_v3(request, &snapshot(&[], &[]))
        .unwrap_err();
    assert!(
        error.to_string().contains("acknowledgement exceeds"),
        "{error}"
    );
    assert_eq!(registry.generation(), 0);
    assert!(registry.registered_definitions().is_empty());
}

#[test]
fn portable_selector_is_requalified_against_the_actual_voices_metadata() {
    let mut value = body();
    value["definitions"][0]["definition"]["choices"][0]["selector"] =
        json!({"kind":"engine_default","engine_id":"dectalk"});
    let mut request = request(&value);
    let EngineRegisteredVoiceDefinition::EngineLayered(definition) = request.definitions.remove(0)
    else {
        panic!()
    };
    let mut inventory = inventory();
    let mut betty = inventory[0].voices[0].clone();
    betty.id.voice_id = "betty".into();
    inventory[0].voices.push(betty);
    inventory[0].default_voice_id = Some("betty".into());
    let mut catalogues = catalogues();
    let mut engine_level = catalogues[0].clone();
    engine_level.voice_id = None;
    for p in &mut engine_level.parameters {
        p.default.source = crate::native_parameters::DefaultSource::Unknown;
        p.default.value = None;
    }
    let mut betty = catalogues[0].clone();
    betty.voice_id = Some("betty".into());
    betty.parameters[1].value_type = crate::native_parameters::ValueType::Integer {
        minimum: 0,
        maximum: 50,
        step: 1,
    };
    catalogues.extend([engine_level, betty]);
    let snapshot = snapshot(&inventory, &catalogues);
    assert_eq!(
        definition.statuses(&snapshot).unwrap()[0].status,
        NativeSupport::Supported
    );
    let mut actual = resolution(0);
    actual.realized.voice_id = "betty".into();
    assert!(definition
        .prepare(
            &actual,
            &VoiceStylePatch::default(),
            0.5,
            None,
            &snapshot,
            UnavailablePolicy::Require
        )
        .is_err());
    assert!(matches!(
        definition
            .prepare(
                &actual,
                &VoiceStylePatch::default(),
                0.5,
                None,
                &snapshot,
                UnavailablePolicy::CommonOnly
            )
            .unwrap()
            .native,
        NativeChoiceExecution::CommonOnly { .. }
    ));
}

#[test]
fn known_schema_rejects_unknown_parameters_invalid_types_and_wrong_scopes_atomically() {
    let (inventory, mut catalogues) = (inventory(), catalogues());
    let mut registry = LogicalVoiceRegistry::default();
    registry
        .register_v3(request(&body()), &snapshot(&inventory, &catalogues))
        .unwrap();
    for native in [
        json!({"sm":{"op":"set","value":false}}),
        json!({"unknown":{"op":"default"}}),
    ] {
        let mut value = body();
        value["registry_generation"] = json!(42);
        value["definitions"][0]["definition"]["choices"][0]["native"]["parameters"] = native;
        assert!(registry
            .register_v3(request(&value), &snapshot(&inventory, &catalogues))
            .is_err());
        assert_eq!(registry.generation(), 41);
    }
    for scope in [ParameterScope::Engine, ParameterScope::Startup] {
        catalogues[0].parameters[1].scope = scope;
        assert!(registry
            .register_v3(request(&body()), &snapshot(&inventory, &catalogues))
            .is_err());
        assert_eq!(registry.generation(), 41);
    }
}
