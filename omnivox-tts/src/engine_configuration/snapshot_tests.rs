use super::*;
use serde_json::json;
use std::path::Path;

fn resolved(managed: Option<&ManagedLaunch>) -> ResolvedConfiguration {
    let root = std::env::temp_dir().join("omnivox absent snapshot fixture");
    let mut loaded = LoadedConfiguration {
        root: Some(root.clone()),
        configuration: Configuration::parse(
        br#"{"schema":1,"routing":{"preferred_engine_ids":[],"disabled_engine_ids":["espeak"],"automatic_engine_ids":["org.fixture"]}}"#,
        Platform::native(),
        ).unwrap(),
        ..LoadedConfiguration::default()
    };
    let manifest = HelperManifest::parse(
        &serde_json::to_vec(&json!({
            "schema":1, "engine_id":"org.fixture", "program": root.join("helper with spaces"),
            "arguments":["", "$(literal)", "private argument"],
            "timeouts":{"startup_ms":120000,"request_ms":100,"synthesis_idle_ms":300000},
        }))
        .unwrap(),
        Platform::native(),
    )
    .unwrap();
    loaded.external.insert(
        "org.fixture".into(),
        ManifestRegistration {
            manifest,
            source: root.join("helpers.d/org.fixture.json"),
        },
    );
    let mut inputs = BTreeMap::new();
    if let Some(managed) = managed {
        inputs.insert(
            "flite".into(),
            RuntimeInputs {
                invocation: Some(RuntimeInvocation::Managed(vec![
                    "--voice-library".into(),
                    managed.path.clone().into_os_string(),
                    "--voice-library-sha256".into(),
                    managed.library.sha256().into(),
                ])),
                ..RuntimeInputs::default()
            },
        );
    }
    ResolvedConfiguration::resolve(
        loaded,
        &root.join("omnivox"),
        Platform::native(),
        LaunchEnvironment::from_variables([
            ("PRIVATE".into(), "private value".into()),
            ("EMPTY".into(), "".into()),
            (
                "OMNIVOX_FLITE_HELPER".into(),
                root.join("flite").into_os_string(),
            ),
        ]),
        &inputs,
    )
    .unwrap()
}

fn prepared() -> LaunchSnapshot {
    LaunchSnapshot::prepare(resolved(None), None, "org.fixture".into(), false).unwrap()
}

fn reject_mutation(snapshot: &LaunchSnapshot, mutate: impl FnOnce(&mut Value)) {
    let mut value = serde_json::to_value(snapshot).unwrap();
    mutate(&mut value);
    let bytes = serde_json::to_vec(&value).unwrap();
    let error = LaunchSnapshot::parse(&bytes)
        .err()
        .expect("invalid snapshot accepted");
    assert!(!error.to_string().contains("private value"));
    assert!(!error.to_string().contains("private argument"));
}

#[test]
fn complete_record_round_trips_without_files_or_environment_reads() {
    let snapshot = prepared();
    let bytes = snapshot.to_bytes().unwrap();
    let decoded = LaunchSnapshot::parse(&bytes).unwrap();
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
    assert_eq!(decoded.activation_id(), snapshot.activation_id());
    assert_ne!(prepared().activation_id(), snapshot.activation_id());
    assert_eq!(decoded.requested(), "org.fixture");
    assert!(!decoded.piper_selected());
    let retained = decoded.resolved();
    assert_eq!(
        retained.environment.get("PRIVATE"),
        Some(OsStr::new("private value"))
    );
    assert_eq!(retained.routing.preferred_engine_ids, Some(Vec::new()));
    assert_eq!(retained.routing.fallback_engine_ids, None);
    assert!(!retained.registration("espeak").unwrap().enabled);
    let external = retained.registration("org.fixture").unwrap();
    assert_eq!(external.origin, EngineOrigin::ExternalHelper);
    let helper = external.helper.as_ref().unwrap();
    assert_eq!(
        helper.arguments,
        ["", "$(literal)", "private argument"].map(OsString::from)
    );
    assert_eq!(helper.environment, retained.environment);
    assert_eq!(helper.startup_timeout, Duration::from_secs(120));
    assert_eq!(helper.request_timeout, Duration::from_millis(100));
    assert_eq!(helper.synthesis_idle_timeout, Duration::from_secs(300));
}

#[test]
fn speech_preferences_survive_complete_snapshot_round_trips() {
    let mut resolved = resolved(None);
    resolved.speech.max_chunk_words = ChunkWordLimit::try_from(30).unwrap();
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let decoded = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.resolved().speech.max_chunk_words.get(), 30);
    for value in [
        json!(null),
        json!({}),
        json!({"max_chunk_words":0}),
        json!({"max_chunk_words":101}),
        json!({"max_chunk_words":30,"extra":true}),
    ] {
        reject_mutation(&snapshot, |wire| wire["speech"] = value);
    }
}

#[test]
fn historical_snapshots_keep_fifteen_words_and_their_old_wire_shape() {
    let mut old = serde_json::to_value(prepared()).unwrap();
    old.as_object_mut().unwrap().remove("punctuation");
    old["schema"] = json!(1);
    old.as_object_mut().unwrap().remove("speech");
    old.as_object_mut().unwrap().remove("speech_defaults");
    old.as_object_mut().unwrap().remove("capital_pitch");
    old.as_object_mut().unwrap().remove("audio");
    let decoded = LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(decoded.resolved().speech.max_chunk_words.get(), 15);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), old);
    old["speech"] = json!({"max_chunk_words":30});
    assert!(LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).is_err());
}

#[test]
fn saved_defaults_are_complete_and_old_chunk_snapshots_keep_their_wire_shape() {
    let mut resolved = resolved(None);
    resolved.speech.defaults.voice = Some("saved voice".into());
    resolved.speech.defaults.rate = 0.7;
    resolved.speech.defaults.split_caps = false;
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let decoded = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.resolved().speech, snapshot.resolved().speech);
    let wire = serde_json::to_value(&snapshot).unwrap();
    for key in wire["speech_defaults"].as_object().unwrap().keys() {
        reject_mutation(&snapshot, |wire| {
            wire["speech_defaults"].as_object_mut().unwrap().remove(key);
        });
    }
    reject_mutation(&snapshot, |wire| wire["speech_defaults"] = json!(null));
    reject_mutation(&snapshot, |wire| {
        wire["speech_defaults"]["pitch"] = json!(0.49)
    });
    reject_mutation(&snapshot, |wire| {
        wire["speech_defaults"]["voice"] = json!("")
    });
    reject_mutation(&snapshot, |wire| {
        wire["speech_defaults"]["extra"] = json!(true)
    });
    for schema in [1, 2] {
        reject_mutation(&snapshot, |wire| wire["schema"] = json!(schema));
    }
    let mut old = wire;
    old.as_object_mut().unwrap().remove("punctuation");
    old["schema"] = json!(2);
    old.as_object_mut().unwrap().remove("speech_defaults");
    old.as_object_mut().unwrap().remove("capital_pitch");
    old.as_object_mut().unwrap().remove("audio");
    old["speech"]["max_chunk_words"] = json!(30);
    let decoded = LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(
        decoded.resolved().speech.defaults,
        SpeechDefaults::default()
    );
    assert_eq!(decoded.resolved().speech.max_chunk_words.get(), 30);
    assert_eq!(serde_json::to_value(&decoded).unwrap(), old);
}

#[test]
fn captured_native_environment_can_prepare_and_restore_a_snapshot() {
    let mut resolved = resolved(None);
    resolved.environment = LaunchEnvironment::capture();
    for registration in resolved.registrations.values_mut() {
        if let Some(helper) = &mut registration.helper {
            helper.environment = resolved.environment.clone();
        }
    }
    let expected = resolved.environment.clone();
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let decoded = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded.resolved.environment, expected);
}

#[test]
fn every_complete_record_field_is_required_even_when_nullable() {
    let snapshot = prepared();
    let value = serde_json::to_value(&snapshot).unwrap();
    for key in value.as_object().unwrap().keys() {
        reject_mutation(&snapshot, |value| {
            value.as_object_mut().unwrap().remove(key);
        });
    }
    for (index, entry) in value["registrations"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        for key in entry.as_object().unwrap().keys() {
            reject_mutation(&snapshot, |value| {
                value["registrations"][index]
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
            });
        }
    }
    for key in value["routing"].as_object().unwrap().keys() {
        reject_mutation(&snapshot, |value| {
            value["routing"].as_object_mut().unwrap().remove(key);
        });
    }
}

#[test]
fn parsing_rejects_duplicate_keys_unknown_fields_and_outer_bounds() {
    let snapshot = prepared();
    let bytes = snapshot.to_bytes().unwrap();
    let mut duplicate = br#"{"sch\u0065ma":1,"#.to_vec();
    duplicate.extend_from_slice(&bytes[1..]);
    assert!(LaunchSnapshot::parse(&duplicate).is_err());
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b" {}");
    assert!(LaunchSnapshot::parse(&trailing).is_err());
    reject_mutation(&snapshot, |value| value["extra"] = json!(true));
    reject_mutation(&snapshot, |value| value["schema"] = json!(8));
    reject_mutation(&snapshot, |value| {
        value["platform"] = json!("another native platform")
    });
    reject_mutation(&snapshot, |value| {
        value["activation_id"] = json!("opaque but not a UUID")
    });
    let mut boundary = bytes;
    boundary.resize(MAX_SNAPSHOT_BYTES, b' ');
    assert!(LaunchSnapshot::parse(&boundary).is_ok());
    boundary.push(b' ');
    assert!(LaunchSnapshot::parse(&boundary).is_err());
    let nested = format!(
        "{}0{}",
        "[".repeat(MAX_JSON_DEPTH + 1),
        "]".repeat(MAX_JSON_DEPTH + 1)
    );
    assert!(LaunchSnapshot::parse(nested.as_bytes()).is_err());
    assert!(Configuration::parse(br#"{"schema":1,"routing":null}"#, Platform::native()).is_err());
}

#[test]
fn parsing_rejects_partial_registration_sets_and_permission_changes() {
    let snapshot = prepared();
    reject_mutation(&snapshot, |value| {
        value["registrations"].as_array_mut().unwrap().remove(0);
    });
    reject_mutation(&snapshot, |value| {
        let entry = value["registrations"][0].clone();
        value["registrations"].as_array_mut().unwrap().push(entry);
    });
    reject_mutation(&snapshot, |value| {
        let entry = value["registrations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["engine_id"] == "espeak")
            .unwrap();
        entry["enabled"] = json!(true);
        entry["unavailable"] = Value::Null;
    });
    reject_mutation(&snapshot, |value| {
        value["routing"]["automatic_engine_ids"] = json!(["org.fixture", "org.fixture"])
    });
    reject_mutation(&snapshot, |value| {
        value["routing"]["preferred_engine_ids"] = json!(["native"])
    });
    for mutation in [
        "origin",
        "source",
        "helper",
        "timeout",
        "arguments",
        "program",
    ] {
        reject_mutation(&snapshot, |value| {
            let entry = value["registrations"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["engine_id"] == "org.fixture")
                .unwrap();
            match mutation {
                "origin" => entry["origin"] = json!("in_process"),
                "source" => entry["source"] = Value::Null,
                "helper" => entry["helper"] = Value::Null,
                "timeout" => entry["helper"]["startup_ms"] = json!(120001),
                "arguments" => {
                    entry["helper"]["arguments"] =
                        serde_json::to_value(vec![OsString::from("x"); 65]).unwrap()
                }
                "program" => {
                    entry["helper"]["program"] =
                        serde_json::to_value(OsStr::new("relative/helper")).unwrap()
                }
                _ => unreachable!(),
            }
        });
    }
}

#[test]
fn environment_rejects_duplicates_and_invalid_native_process_values() {
    let snapshot = prepared();
    for variables in [
        vec![("DUP", "a"), ("DUP", "b")],
        vec![("", "value")],
        vec![("KEY=VALUE", "value")],
        vec![("=C:", "C:\\private")],
        vec![("KEY\0", "value")],
        vec![("KEY", "value\0")],
    ] {
        reject_mutation(&snapshot, |value| {
            value["environment"] = serde_json::to_value(
                variables
                    .into_iter()
                    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        });
    }
    #[cfg(windows)]
    reject_mutation(&snapshot, |value| {
        value["environment"] = serde_json::to_value(vec![
            (OsString::from("Path"), OsString::from("a")),
            (OsString::from("PATH"), OsString::from("b")),
        ])
        .unwrap()
    });
}

#[test]
fn preparation_does_not_silently_replace_an_inconsistent_environment() {
    let mut resolved = resolved(None);
    resolved
        .registrations
        .get_mut("org.fixture")
        .unwrap()
        .helper
        .as_mut()
        .unwrap()
        .environment =
        LaunchEnvironment::from_variables([("PRIVATE".into(), "different value".into())]);
    assert!(LaunchSnapshot::prepare(resolved, None, "".into(), false).is_err());
}

#[test]
fn managed_generation_keeps_exact_bytes_hash_and_provider_owned_arguments() {
    let generation = br#"{ "schema_version":1, "target_id":"11111111-1111-4111-8111-111111111111",
      "profile_id":"22222222-2222-4222-8222-222222222222", "generation_id":"33333333-3333-4333-8333-333333333333",
      "disabled_physical_ids":[], "piper":null, "flite":{"builtin_slt":true,"files":[]} }"#;
    let managed = ManagedLaunch {
        path: std::env::temp_dir().join("missing managed generation.json"),
        library: RuntimeLibrary::parse(
            generation,
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )
        .unwrap(),
        overrides: ProviderOverrides::default(),
    };
    let expected = managed.library.configuration();
    let snapshot =
        LaunchSnapshot::prepare(resolved(Some(&managed)), Some(managed), "".into(), false).unwrap();
    let decoded = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(
        decoded.managed().unwrap().library.source_bytes(),
        generation
    );
    assert_eq!(decoded.managed().unwrap().library.configuration(), expected);
    reject_mutation(&snapshot, |value| {
        let entry = value["registrations"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["engine_id"] == "flite")
            .unwrap();
        entry["helper"]["arguments"] = json!([]);
    });
    reject_mutation(&snapshot, |value| {
        value["managed"]["generation"] = json!("{}")
    });
    assert!(Path::new(&decoded.managed().unwrap().path).is_absolute());
}

#[cfg(any(unix, windows))]
#[test]
fn native_non_unicode_environment_and_legacy_arguments_survive_handoff() {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;
    #[cfg(unix)]
    let native = OsString::from_vec(vec![b'a', 0xff, b'b']);
    #[cfg(windows)]
    let native = OsString::from_wide(&[b'a' as u16, 0xd800, b'b' as u16]);
    let mut resolved = resolved(None);
    resolved.environment = LaunchEnvironment::from_variables([(native.clone(), native.clone())]);
    for registration in resolved.registrations.values_mut() {
        if let Some(helper) = &mut registration.helper {
            helper.environment = resolved.environment.clone();
            if registration.origin == EngineOrigin::ShippedHelper {
                helper.arguments = vec![native.clone()];
            }
        }
    }
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let decoded = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(
        decoded.resolved.environment.variables().next().unwrap(),
        (&native, &native)
    );
    assert_eq!(
        decoded
            .resolved
            .registration("flite")
            .unwrap()
            .helper
            .as_ref()
            .unwrap()
            .arguments,
        vec![native]
    );
}

#[test]
fn capital_pitch_is_frozen_and_schema_three_retains_its_old_shape() {
    use omnivox_core::settings::CapitalPitch;
    let mut resolved = resolved(None);
    resolved.speech.capital_pitch.default = CapitalPitch::Off;
    resolved
        .speech
        .capital_pitch
        .engines
        .insert("org.fixture".into(), CapitalPitch::Value(1.7));
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let restored = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(restored.resolved().speech, snapshot.resolved().speech);
    for value in [
        json!(null),
        json!({}),
        json!({"default":1.5}),
        json!({"engines":{}}),
        json!({"default":0,"engines":{}}),
        json!({"default":1.5,"engines":{"unregistered":1.5}}),
        json!({"default":1.5,"engines":{},"extra":true}),
    ] {
        reject_mutation(&snapshot, |wire| wire["capital_pitch"] = value);
    }
    reject_mutation(&snapshot, |wire| {
        wire.as_object_mut().unwrap().remove("capital_pitch");
    });
    for schema in 1..=3 {
        reject_mutation(&snapshot, |wire| wire["schema"] = json!(schema));
    }
    let mut old = serde_json::to_value(snapshot).unwrap();
    old.as_object_mut().unwrap().remove("punctuation");
    old["schema"] = json!(3);
    old.as_object_mut().unwrap().remove("capital_pitch");
    old.as_object_mut().unwrap().remove("audio");
    let restored = LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(
        restored.resolved().speech.capital_pitch,
        CapitalPitchSettings::default()
    );
    assert_eq!(serde_json::to_value(restored).unwrap(), old);
}

#[test]
fn audio_is_complete_frozen_and_old_snapshots_keep_their_shape() {
    let mut resolved = resolved(None);
    resolved.audio = AudioOutputSettings {
        backend: omnivox_core::settings::AudioBackend::Null,
        target: omnivox_core::state::ChannelMode::Left,
        pulse_latency_ms: omnivox_core::settings::PulseLatencyMs::try_from(45).unwrap(),
    };
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let restored = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(restored.resolved().audio, snapshot.resolved().audio);
    for key in ["backend", "target", "pulse_latency_ms"] {
        reject_mutation(&snapshot, |wire| {
            wire["audio"].as_object_mut().unwrap().remove(key);
        });
    }
    for audio in [
        json!(null),
        json!({}),
        json!({"backend":"device","target":"both","pulse_latency_ms":201}),
        json!({"backend":"unknown","target":"both","pulse_latency_ms":20}),
    ] {
        reject_mutation(&snapshot, |wire| wire["audio"] = audio);
    }
    reject_mutation(&snapshot, |wire| {
        wire.as_object_mut().unwrap().remove("audio");
    });
    for schema in 1..=4 {
        reject_mutation(&snapshot, |wire| wire["schema"] = json!(schema));
    }
    let mut old = serde_json::to_value(snapshot).unwrap();
    old.as_object_mut().unwrap().remove("punctuation");
    old["schema"] = json!(4);
    old.as_object_mut().unwrap().remove("audio");
    let restored = LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(restored.resolved().audio, AudioOutputSettings::default());
    assert_eq!(serde_json::to_value(restored).unwrap(), old);
}

#[test]
fn punctuation_is_complete_frozen_and_historical_schemas_keep_ascii_behavior() {
    let mut resolved = resolved(None);
    resolved
        .speech
        .punctuation
        .some
        .insert('’', Some("single quote".into()));
    resolved.speech.punctuation.all.insert('$', None);
    let snapshot = LaunchSnapshot::prepare(resolved, None, "".into(), false).unwrap();
    let restored = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
    assert_eq!(
        restored.resolved().speech.punctuation,
        snapshot.resolved().speech.punctuation
    );
    for value in [
        json!(null),
        json!({}),
        json!({"none":{},"some":{}}),
        json!({"none":{},"some":{},"all":{},"custom":{}}),
        json!({"none":{},"some":{"’":""},"all":{}}),
        json!({"none":{},"some":{"ab":"invalid"},"all":{}}),
    ] {
        reject_mutation(&snapshot, |wire| wire["punctuation"] = value);
    }
    let mut old = serde_json::to_value(&snapshot).unwrap();
    old["schema"] = json!(5);
    assert!(LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).is_err());
    old.as_object_mut().unwrap().remove("punctuation");
    let restored = LaunchSnapshot::parse(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(
        restored.resolved().speech.punctuation,
        PunctuationTables::legacy()
    );
    assert_eq!(serde_json::to_value(restored).unwrap(), old);
}

#[test]
fn named_profiles_are_frozen_and_rejected_by_historical_snapshot_shapes() {
    let mut settings = resolved(None);
    settings.speech = Configuration::parse(br#"{"schema":4,"speech":{"punctuation_profiles":{"prose":{"base":"some","overrides":{"!":"notice"}}}}}"#, Platform::native()).unwrap().speech;
    let snapshot = LaunchSnapshot::prepare(settings, None, "org.fixture".into(), false).unwrap();
    let encoded = serde_json::to_vec(&snapshot).unwrap();
    let restored = LaunchSnapshot::parse(&encoded).unwrap();
    assert_eq!(
        restored.resolved.speech.punctuation.profiles["prose"].table[&'!'].as_deref(),
        Some("notice")
    );
    reject_mutation(&snapshot, |value| value["schema"] = json!(6));
}
