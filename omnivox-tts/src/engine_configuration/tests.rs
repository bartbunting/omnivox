use super::*;
use serde_json::json;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn manifest(id: &str) -> Value {
    json!({"schema":1,"engine_id":id,"program": if cfg!(windows) { r"C:\Speech Files\helper.exe" } else { "/opt/Speech Files/helper" }})
}

fn parse_manifest(value: &Value) -> Result<HelperManifest> {
    HelperManifest::parse(&serde_json::to_vec(value).unwrap(), Platform::native())
}

fn parse_config(value: &Value) -> Result<Configuration> {
    Configuration::parse(&serde_json::to_vec(value).unwrap(), Platform::native())
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("omnivox-config-{}-{time}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn helpers(&self) -> PathBuf {
        let path = self.0.join("helpers.d");
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn write(&self, name: &str, value: &Value) {
        fs::write(
            self.helpers().join(name),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn load(&self) -> Result<LoadedConfiguration> {
        ConfigurationRoot {
            path: self.0.clone(),
            explicit: true,
        }
        .load()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn omitted_and_empty_routing_lists_have_distinct_meanings() {
    let default = parse_config(&json!({"schema":1})).unwrap();
    assert!(default == Configuration::default());
    let empty = parse_config(
        &json!({"schema":1,"routing":{"preferred_engine_ids":[],"fallback_engine_ids":[]}}),
    )
    .unwrap();
    assert_eq!(empty.routing.preferred_engine_ids, Some(vec![]));
    assert_eq!(empty.routing.fallback_engine_ids, Some(vec![]));
    let configured = parse_config(&json!({"schema":1,"routing":{"preferred_engine_ids":["future.engine","espeak"],"disabled_engine_ids":["future.engine"]}})).unwrap();
    assert_eq!(
        configured.routing.preferred_engine_ids.unwrap(),
        ["future.engine", "espeak"]
    );
}

#[test]
fn speech_preferences_require_schema_two_and_a_bounded_integer() {
    for value in [
        json!({"schema":1}),
        json!({"schema":2}),
        json!({"schema":2,"speech":{}}),
    ] {
        assert_eq!(
            parse_config(&value).unwrap().speech.max_chunk_words.get(),
            15
        );
    }
    for limit in [1, 3, 15, 30, 100] {
        let value = json!({"schema":2,"speech":{"max_chunk_words":limit}});
        assert_eq!(
            parse_config(&value).unwrap().speech.max_chunk_words.get(),
            limit
        );
    }
    for limit in [
        json!(0),
        json!(101),
        json!(-1),
        json!(15.0),
        json!("15"),
        json!(null),
        json!(65536),
    ] {
        assert!(parse_config(&json!({"schema":2,"speech":{"max_chunk_words":limit}})).is_err());
    }
    for value in [
        json!({"schema":1,"speech":{"max_chunk_words":30}}),
        json!({"schema":2,"speech":null}),
        json!({"schema":2,"speech":{"unexpected":true}}),
    ] {
        assert!(parse_config(&value).is_err());
    }
}

#[test]
fn speech_defaults_are_sparse_bounded_and_strict() {
    let parse = |defaults: Value| parse_config(&json!({"schema":2,"speech":{"defaults":defaults}}));
    assert_eq!(
        parse(json!({})).unwrap().speech.defaults,
        SpeechDefaults::default()
    );
    for (field, min, max) in [
        ("rate", 0.0, 2.0),
        ("pitch", 0.5, 2.0),
        ("voice_volume", 0.0, 1.0),
        ("tone_volume", 0.0, 1.0),
        ("sound_volume", 0.0, 1.0),
        ("character_scale", 0.1, 4.0),
    ] {
        for value in [json!(min), json!(max)] {
            assert!(parse(json!({field:value})).is_ok(), "{field}");
        }
        for value in [
            json!(min - 0.01),
            json!(max + 0.01),
            json!("0.5"),
            json!(true),
            json!(null),
            json!(1e300),
        ] {
            assert!(parse(json!({field:value})).is_err(), "{field}");
        }
    }
    let saved =
        parse(json!({"voice":"Žltý voice", "rate":0.7, "split_caps":false, "punctuation":"some"}))
            .unwrap();
    assert_eq!(saved.speech.defaults.voice.as_deref(), Some("Žltý voice"));
    assert_eq!(saved.speech.defaults.rate, 0.7);
    assert!(!saved.speech.defaults.split_caps);
    assert_eq!(saved.speech.defaults.pitch, 1.0);
    for value in [
        json!({"voice":""}),
        json!({"voice":"  "}),
        json!({"voice":"secret\nvoice"}),
        json!({"voice":"x".repeat(1025)}),
        json!({"voice":null}),
        json!({"punctuation":"ALL"}),
        json!({"split_caps":1}),
        json!({"unknown":0}),
        json!(null),
    ] {
        assert!(parse(value).is_err());
    }
    assert!(Configuration::parse(
        br#"{"schema":2,"speech":{"defaults":{"rate":0.5,"rate":0.7}}}"#,
        Platform::Unix
    )
    .is_err());
    assert!(parse_config(&json!({"schema":1,"speech":{"defaults":{}}})).is_err());
}

#[test]
fn shipped_metadata_preserves_platform_paths_and_explicit_overrides() {
    let directory = Directory::new();
    let executable = directory.0.join("omnivox");
    let definition = shipped::definition("rhvoice").unwrap();
    let environment = LaunchEnvironment::from_variables([]);
    let candidates = definition.helper_candidates(Platform::Unix);
    let companion = directory.0.join(&candidates[0]);
    let adjacent = directory.0.join(&candidates[1]);
    fs::create_dir_all(companion.parent().unwrap()).unwrap();
    fs::write(&companion, b"companion").unwrap();
    fs::write(&adjacent, b"legacy").unwrap();
    assert_eq!(
        definition
            .helper_config(&executable, Platform::Unix, &environment)
            .unwrap()
            .program,
        companion
    );
    let missing_override = directory.0.join("explicit missing helper");
    let environment = LaunchEnvironment::from_variables([(
        "OMNIVOX_RHVOICE_HELPER".into(),
        missing_override.clone().into_os_string(),
    )]);
    assert_eq!(
        definition
            .helper_config(&executable, Platform::Unix, &environment)
            .unwrap()
            .program,
        missing_override
    );
    let environment =
        LaunchEnvironment::from_variables([("OMNIVOX_RHVOICE_HELPER".into(), "".into())]);
    assert_eq!(
        definition
            .helper_config(&executable, Platform::Unix, &environment)
            .unwrap()
            .program,
        companion
    );
    assert_eq!(
        definition.helper_candidates(Platform::Windows),
        [
            PathBuf::from("rhvoice").join("omnivox-rhvoice-helper.exe"),
            PathBuf::from("omnivox-rhvoice-helper.exe")
        ]
    );
    assert_eq!(
        shipped::definition("eloquence")
            .unwrap()
            .helper_candidates(Platform::Windows),
        [PathBuf::from("OmnivoxEloquenceHelper32.exe")]
    );
    assert_eq!(
        shipped::definition("dectalk")
            .unwrap()
            .helper_candidates(Platform::Windows),
        [PathBuf::from("OmnivoxDectalkHelper32.exe")]
    );
    assert!(shipped::definition("eloquence")
        .unwrap()
        .helper_candidates(Platform::MacOs)
        .is_empty());
    assert!(shipped::definition("mbrola")
        .unwrap()
        .helper_candidates(Platform::Unix)
        .is_empty());
    assert!(shipped::definition("espeak")
        .unwrap()
        .helper_config(&executable, Platform::Unix, &environment)
        .is_none());
}

#[test]
fn strict_json_rejects_ambiguous_input_without_disclosing_values() {
    for bytes in [
        br#"{"schema":1,"schema":1}"#.as_slice(),
        br#"{"schema":1,"\u0073chema":1}"#,
        br#"{"schema":1,"routing":{"fallback_engine_ids":[],"fallback_engine_ids":[]}}"#,
        br#"{"schema":1,"engine_overrides":{"espeak":{},"\u0065speak":{}}}"#,
        br#"{"schema":1,"routing":null}"#,
        br#"{"schema":1,"routing":{"preferred_engine_ids":null}}"#,
        br#"{"schema":1,"engine_overrides":{"piper":{"enabled":null}}}"#,
        br#"{"schema":1,}"#,
        br#"{"schema":1}{}"#,
        br#"/* comment */{"schema":1}"#,
        br#"{"schema":"secret-value"}"#,
        br#"{"schema":1.0}"#,
        br#"{"schema":1e0}"#,
        br#"{"schema":18446744073709551616}"#,
        br#"{"schema":-1}"#,
        br#"{"schema":4}"#,
        br#"{}"#,
        br#"[]"#,
        b"\xff",
        b"\xef\xbb\xbf\xef\xbb\xbf{}",
    ] {
        let error = Configuration::parse(bytes, Platform::Unix)
            .err()
            .expect("must reject");
        assert!(!error.to_string().contains("secret-value"));
    }
    assert!(Configuration::parse(b"\xef\xbb\xbf{\"schema\":1}", Platform::Unix).is_ok());
}

#[test]
fn unknown_fields_fail_at_every_level_including_future_language_policy() {
    for value in [
        json!({"schema":1,"default_language":"en"}),
        json!({"schema":1,"routing":{"language_rules":[]}}),
        json!({"schema":1,"routing":{"cross_language_fallback":true}}),
        json!({"schema":1,"engine_overrides":{"piper":{"capabilities":{}}}}),
        json!({"schema":1,"engine_overrides":{"piper":{"timeouts":{"cancellation_ms":100}}}}),
    ] {
        assert!(parse_config(&value).is_err());
    }
    for key in [
        "environment",
        "working_directory",
        "voices",
        "capabilities",
        "descriptor_cache",
    ] {
        let mut value = manifest("example");
        value[key] = json!({});
        assert!(parse_manifest(&value).is_err());
    }
}

#[test]
fn file_and_nesting_bounds_include_bom_and_invalid_values() {
    for (maximum, is_manifest) in [(MAX_CONFIG_BYTES, false), (MAX_MANIFEST_BYTES, true)] {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(
            serde_json::to_vec(&if is_manifest {
                manifest("example")
            } else {
                json!({"schema":1})
            })
            .unwrap(),
        );
        bytes.resize(maximum, b' ');
        let parse = |bytes: &[u8]| {
            if is_manifest {
                HelperManifest::parse(bytes, Platform::native()).map(|_| ())
            } else {
                Configuration::parse(bytes, Platform::native()).map(|_| ())
            }
        };
        assert!(parse(&bytes).is_ok());
        bytes.push(b' ');
        assert!(parse(&bytes).is_err());
    }
    for depth in [MAX_JSON_DEPTH, MAX_JSON_DEPTH + 1] {
        let bytes = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            json::parse(bytes.as_bytes(), MAX_CONFIG_BYTES).is_ok(),
            depth == MAX_JSON_DEPTH
        );
    }
}

#[test]
fn manifest_identity_and_literal_arguments_are_validated() {
    for id in ["a", "org.example-voice_1", &"a".repeat(128)] {
        assert!(parse_manifest(&manifest(id)).is_ok());
    }
    for id in [
        "",
        "Upper",
        "1engine",
        "a..b",
        "a-_b",
        "a.",
        "a/b",
        "a b",
        "é",
        &"a".repeat(129),
    ] {
        assert!(parse_manifest(&manifest(id)).is_err(), "{id}");
    }
    let mut value = manifest("example");
    value["arguments"] = json!([
        "",
        "two words",
        "$HOME",
        "$(private)",
        "`private`",
        "*",
        "--runtime",
        "x\ny"
    ]);
    let parsed = parse_manifest(&value).unwrap();
    assert_eq!(
        parsed.arguments,
        [
            "",
            "two words",
            "$HOME",
            "$(private)",
            "`private`",
            "*",
            "--runtime",
            "x\ny"
        ]
    );
    assert!(parsed.enabled);
    for (arguments, valid) in [
        (vec!["".to_owned(); 64], true),
        (vec!["".to_owned(); 65], false),
        (vec!["x".repeat(4096); 4], true),
        (vec!["x".repeat(4096); 5], false),
        (vec!["x".repeat(4097)], false),
        (vec!["é".repeat(2048)], true),
        (vec!["é".repeat(2049)], false),
        (vec!["private\0value".into()], false),
    ] {
        value["arguments"] = json!(arguments);
        assert_eq!(parse_manifest(&value).is_ok(), valid);
    }
}

#[test]
fn native_paths_are_checked_independently_of_test_platform() {
    for (platform, valid, invalid) in [
        (
            Platform::Unix,
            vec![
                "/helper",
                "/space directory/helper",
                "//server/share/helper",
            ],
            vec!["helper", "~/helper", "$HOME/helper", r"C:\helper", ""],
        ),
        (
            Platform::MacOs,
            vec!["/Applications/Speech Helper"],
            vec!["./helper"],
        ),
        (
            Platform::Windows,
            vec![
                r"C:\helper.exe",
                "C:/space dir/helper.exe",
                r"\\server\share\helper.exe",
                "//server/share/helper.exe",
            ],
            vec![
                r"C:helper.exe",
                r"\helper.exe",
                "/helper.exe",
                r"\\?\C:\helper.exe",
                r"\\.\pipe\helper",
                r"\??\C:\helper",
                "//?/C:/helper",
                r"\\server",
                r"\\server\\helper",
            ],
        ),
    ] {
        for path in valid {
            assert!(platform.validate_path(path, "path").is_ok(), "{path}");
        }
        for path in invalid {
            assert!(platform.validate_path(path, "path").is_err(), "{path}");
        }
    }
    assert!(Platform::Unix
        .validate_path(&format!("/{}", "x".repeat(4095)), "path")
        .is_ok());
    assert!(Platform::Unix
        .validate_path(&format!("/{}", "x".repeat(4096)), "path")
        .is_err());
    for path in ["/a\nb", "/a\0b", "/a\u{7f}"] {
        assert!(Platform::Unix.validate_path(path, "path").is_err());
    }
}

#[test]
fn root_precedence_and_native_defaults_do_not_merge() {
    let environment = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>()
    };
    for (platform, pairs, expected) in [
        (
            Platform::Unix,
            vec![("HOME", "/home/u")],
            "/home/u/.config/omnivox",
        ),
        (
            Platform::Unix,
            vec![("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/settings")],
            "/settings/omnivox",
        ),
        (
            Platform::Unix,
            vec![("HOME", "/home/u"), ("XDG_CONFIG_HOME", "")],
            "/home/u/.config/omnivox",
        ),
        (
            Platform::MacOs,
            vec![("HOME", "/Users/u"), ("XDG_CONFIG_HOME", "relative")],
            "/Users/u/Library/Application Support/Omnivox",
        ),
        (
            Platform::Windows,
            vec![
                ("APPDATA", r"C:\Users\u\AppData\Roaming"),
                ("HOME", "/home/u"),
            ],
            r"C:\Users\u\AppData\Roaming\omnivox",
        ),
    ] {
        let pairs = environment(&pairs);
        let root = platform
            .configuration_root(None, |key| pairs.get(key).map(Into::into))
            .unwrap();
        assert_eq!(root.path, PathBuf::from(expected));
        assert!(!root.explicit);
    }
    let env = environment(&[("OMNIVOX_CONFIG_DIR", "/env"), ("HOME", "relative")]);
    assert_eq!(
        Platform::Unix
            .configuration_root(Some(OsStr::new("/cli")), |key| env.get(key).map(Into::into))
            .unwrap()
            .path,
        PathBuf::from("/cli")
    );
    assert_eq!(
        Platform::Unix
            .configuration_root(None, |key| env.get(key).map(Into::into))
            .unwrap()
            .path,
        PathBuf::from("/env")
    );
    for explicit in ["", "relative", "~/settings"] {
        assert!(Platform::Unix
            .configuration_root(Some(OsStr::new(explicit)), |_| None)
            .is_err());
    }
    for pairs in [
        vec![],
        vec![("HOME", "")],
        vec![("HOME", "relative")],
        vec![("HOME", "/valid"), ("XDG_CONFIG_HOME", "relative")],
        vec![("HOME", "/valid"), ("OMNIVOX_CONFIG_DIR", "relative")],
    ] {
        let pairs = environment(&pairs);
        assert!(Platform::Unix
            .configuration_root(None, |key| pairs.get(key).map(Into::into))
            .is_err());
    }
}

#[test]
fn explicit_root_precedence_and_errors_apply_on_every_platform() {
    for (platform, cli, env, base) in [
        (Platform::Unix, "/cli", "/environment", "HOME"),
        (Platform::MacOs, "/cli", "/environment", "HOME"),
        (Platform::Windows, r"C:\cli", r"C:\environment", "APPDATA"),
    ] {
        let variables = |key: &str| match key {
            "OMNIVOX_CONFIG_DIR" => Some(env.into()),
            key if key == base => Some("invalid relative base".into()),
            _ => None,
        };
        let root = platform
            .configuration_root(Some(OsStr::new(cli)), variables)
            .unwrap();
        assert_eq!(root.path, PathBuf::from(cli));
        assert!(root.explicit);
        let root = platform.configuration_root(None, variables).unwrap();
        assert_eq!(root.path, PathBuf::from(env));
        assert!(root.explicit);
        for invalid in ["", "relative", "~/settings"] {
            assert!(platform
                .configuration_root(Some(OsStr::new(invalid)), variables)
                .is_err());
        }
        for invalid in [None, Some(""), Some("relative")] {
            assert!(platform
                .configuration_root(None, |key| {
                    (key == base).then(|| invalid.map(Into::into)).flatten()
                })
                .is_err());
        }
        assert!(platform
            .configuration_root(None, |key| match key {
                "OMNIVOX_CONFIG_DIR" => Some("relative".into()),
                key if key == base => Some(cli.into()),
                _ => None,
            })
            .is_err());
        let root = platform
            .configuration_root(None, |key| match key {
                "OMNIVOX_CONFIG_DIR" => Some("".into()),
                key if key == base => Some(cli.into()),
                _ => None,
            })
            .unwrap();
        assert!(!root.explicit);
    }
}

#[test]
fn timeout_overrides_are_unsigned_bounded_and_partial() {
    for (name, maximum) in [
        ("startup_ms", 120_000),
        ("request_ms", 30_000),
        ("synthesis_idle_ms", 300_000),
    ] {
        for (number, valid) in [
            (99, false),
            (100, true),
            (maximum, true),
            (maximum + 1, false),
        ] {
            let mut value = manifest("example");
            value["timeouts"] = json!({name:number});
            assert_eq!(parse_manifest(&value).is_ok(), valid);
        }
        for number in [
            json!(100.0),
            json!("100"),
            json!(-100),
            Value::Null,
            json!(true),
        ] {
            let mut value = manifest("example");
            value["timeouts"] = json!({name:number});
            assert!(parse_manifest(&value).is_err());
        }
    }
    let configuration = parse_config(&json!({"schema":1,"engine_overrides":{"piper":{"arguments":[],"timeouts":{"request_ms":100}}}})).unwrap();
    let value = &configuration.engine_overrides["piper"];
    assert_eq!(value.arguments, Some(vec![]));
    assert_eq!(value.timeouts.as_ref().unwrap().startup_ms, None);
    assert_eq!(value.timeouts.as_ref().unwrap().request_ms, Some(100));
}

#[test]
fn override_references_and_in_process_restrictions_fail_closed() {
    for extra in [
        json!({"program":"/helper"}),
        json!({"arguments":[]}),
        json!({"timeouts":{}}),
    ] {
        let config = Configuration::parse(
            serde_json::to_string(&json!({"schema":1,"engine_overrides":{"espeak":extra}}))
                .unwrap()
                .as_bytes(),
            Platform::Unix,
        )
        .unwrap();
        assert!(config.validate_overrides(&BTreeSet::new()).is_err());
    }
    let config = parse_config(&json!({"schema":1,"engine_overrides":{"espeak":{"enabled":false},"org.example":{"enabled":true}}})).unwrap();
    assert!(config.validate_overrides(&BTreeSet::new()).is_err());
    assert!(config
        .validate_overrides(&BTreeSet::from(["org.example".into()]))
        .is_ok());
    for key in [
        "preferred_engine_ids",
        "fallback_engine_ids",
        "disabled_engine_ids",
        "automatic_engine_ids",
    ] {
        for ids in [
            json!(["a", "a"]),
            json!(["native"]),
            json!([""]),
            json!(["a b"]),
            json!(["x".repeat(129)]),
        ] {
            assert!(parse_config(&json!({"schema":1,"routing":{key:ids}})).is_err());
        }
        for count in [64, 65] {
            let ids: Vec<_> = (0..count).map(|i| format!("engine{i}")).collect();
            assert_eq!(
                parse_config(&json!({"schema":1,"routing":{key:ids}})).is_ok(),
                count == 64
            );
        }
    }
    for count in [64, 65] {
        let entries: Map<String, Value> = (0..count)
            .map(|i| (format!("engine{i}"), json!({})))
            .collect();
        assert_eq!(
            parse_config(&json!({"schema":1,"engine_overrides":entries})).is_ok(),
            count == 64
        );
    }
}

#[test]
fn optional_roots_and_missing_files_preserve_defaults_without_writes() {
    let directory = Directory::new();
    let missing = directory.0.join("absent");
    assert!(ConfigurationRoot {
        path: missing.clone(),
        explicit: false
    }
    .load()
    .is_ok());
    assert!(ConfigurationRoot {
        path: missing.clone(),
        explicit: true
    }
    .load()
    .is_err());
    assert!(!missing.exists());
    let loaded = directory.load().unwrap();
    assert!(loaded.configuration == Configuration::default());
    assert!(loaded.external.is_empty());
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
    fs::write(directory.0.join("config.json"), br#"{"schema":1}"#).unwrap();
    assert!(directory.load().unwrap().configuration == Configuration::default());
    fs::write(directory.0.join("config.json"), br#"{"schema":4}"#).unwrap();
    assert!(directory.load().is_err());
    fs::remove_file(directory.0.join("config.json")).unwrap();
    fs::create_dir(directory.0.join("config.json")).unwrap();
    assert!(directory.load().is_err());
}

#[test]
fn manifest_order_collisions_and_invalid_files_preserve_unrelated_engines() {
    let directory = Directory::new();
    directory.write("z.json", &manifest("z.engine"));
    directory.write("a.json", &manifest("a.engine"));
    directory.write("#ignored#", &manifest("ignored"));
    fs::write(directory.helpers().join("bad.json"), "private invalid text").unwrap();
    let loaded = directory.load().unwrap();
    assert_eq!(
        loaded
            .external
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["a.engine", "z.engine"]
    );
    assert_eq!(loaded.diagnostics.len(), 1);
    let mut duplicate = manifest("a.engine");
    duplicate["enabled"] = json!(false);
    directory.write("duplicate.json", &duplicate);
    for engine in shipped::ENGINES {
        directory.write(&format!("{}.json", engine.id), &manifest(engine.id));
    }
    directory.write("native.json", &manifest("native"));
    let loaded = directory.load().unwrap();
    assert_eq!(
        loaded
            .external
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["z.engine"]
    );
    assert!(!loaded
        .diagnostics
        .iter()
        .any(|d| d.to_string().contains("private invalid")));
    fs::write(
        directory.0.join("config.json"),
        br#"{"schema":1,"engine_overrides":{"a.engine":{"enabled":true}}}"#,
    )
    .unwrap();
    assert!(directory.load().is_err());
}

#[test]
fn candidate_and_aggregate_limits_reject_entire_external_set() {
    let directory = Directory::new();
    for index in 0..MAX_MANIFESTS {
        let mut value = manifest(&format!("engine{index}"));
        value["enabled"] = json!(false);
        directory.write(&format!("{index:02}.json"), &value);
    }
    assert_eq!(directory.load().unwrap().external.len(), MAX_MANIFESTS);
    fs::write(directory.helpers().join("invalid.json"), "broken").unwrap();
    let loaded = directory.load().unwrap();
    assert!(loaded.external.is_empty());
    assert_eq!(loaded.diagnostics.len(), 1);
    fs::write(
        directory.0.join("config.json"),
        br#"{"schema":1,"engine_overrides":{"engine0":{}}}"#,
    )
    .unwrap();
    assert!(directory.load().is_err());

    let directory = Directory::new();
    for index in 0..16 {
        let mut bytes = serde_json::to_vec(&manifest(&format!("engine{index}"))).unwrap();
        bytes.resize(MAX_MANIFEST_BYTES, b' ');
        fs::write(directory.helpers().join(format!("{index}.json")), bytes).unwrap();
    }
    assert_eq!(directory.load().unwrap().external.len(), 16);
    fs::write(directory.helpers().join("additional.json"), b" ").unwrap();
    assert!(directory.load().unwrap().external.is_empty());

    let directory = Directory::new();
    directory.write("valid.json", &manifest("valid"));
    fs::write(
        directory.helpers().join("large.json"),
        vec![b' '; MAX_MANIFEST_BYTES + 1],
    )
    .unwrap();
    assert_eq!(directory.load().unwrap().external.len(), 1);
    fs::write(
        directory.helpers().join("large.json"),
        vec![b' '; MAX_MANIFEST_TOTAL_BYTES + 1],
    )
    .unwrap();
    assert!(directory.load().unwrap().external.is_empty());
}

#[test]
fn helper_directory_failure_preserves_policy_but_invalidates_dependent_override() {
    let directory = Directory::new();
    fs::write(directory.0.join("helpers.d"), b"not a directory").unwrap();
    fs::write(
        directory.0.join("config.json"),
        br#"{"schema":1,"routing":{"disabled_engine_ids":["piper"]}}"#,
    )
    .unwrap();
    let loaded = directory.load().unwrap();
    assert_eq!(loaded.configuration.routing.disabled_engine_ids, ["piper"]);
    assert!(loaded.external.is_empty());
    assert_eq!(loaded.diagnostics.len(), 1);
    fs::write(
        directory.0.join("config.json"),
        br#"{"schema":1,"engine_overrides":{"external":{}}}"#,
    )
    .unwrap();
    assert!(directory.load().is_err());
}

#[cfg(unix)]
#[test]
fn redirected_root_is_supported_but_linked_and_special_manifests_are_not_read() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let directory = Directory::new();
    let outer = Directory::new();
    symlink(&directory.0, outer.0.join("redirected")).unwrap();
    directory.write("valid.json", &manifest("valid"));
    symlink(
        directory.helpers().join("valid.json"),
        directory.helpers().join("linked.json"),
    )
    .unwrap();
    fs::create_dir(directory.helpers().join("directory.json")).unwrap();
    let fifo = directory.helpers().join("fifo.json");
    use std::os::unix::ffi::OsStrExt;
    let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let root = ConfigurationRoot {
        path: outer.0.join("redirected"),
        explicit: true,
    };
    let loaded = root.load().unwrap();
    assert_eq!(loaded.external.len(), 1);
    assert_eq!(loaded.diagnostics.len(), 3);
    fs::set_permissions(
        directory.helpers().join("valid.json"),
        fs::Permissions::from_mode(0o0),
    )
    .unwrap();
    if unsafe { libc::geteuid() } != 0 {
        assert!(root.load().unwrap().external.is_empty());
    }
    fs::set_permissions(
        directory.helpers().join("valid.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o0)).unwrap();
    let unreadable = root.load();
    fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).unwrap();
    if unsafe { libc::geteuid() } != 0 {
        assert!(unreadable.is_err());
    }
    symlink(outer.0.join("missing"), outer.0.join("dangling")).unwrap();
    assert!(ConfigurationRoot {
        path: outer.0.join("dangling"),
        explicit: false
    }
    .load()
    .is_err());
}

#[cfg(windows)]
#[test]
fn redirected_windows_root_is_supported_but_manifest_junctions_are_rejected() {
    use std::path::Path;
    use std::process::Command;

    // Junctions exercise real Windows reparse points without requiring the
    // administrator/developer-mode privilege needed to create file symlinks.
    fn junction(link: &Path, target: &Path) {
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {output:?}"
        );
    }

    let directory = Directory::new();
    let outer = Directory::new();
    directory.write("valid.json", &manifest("valid"));
    junction(&outer.0.join("redirected"), &directory.0);
    junction(&directory.helpers().join("linked.json"), &outer.0);
    let root = ConfigurationRoot {
        path: outer.0.join("redirected"),
        explicit: true,
    };
    let loaded = root.load().unwrap();
    assert_eq!(loaded.external.keys().collect::<Vec<_>>(), [&"valid"]);
    assert_eq!(loaded.diagnostics.len(), 1);
    assert!(loaded.diagnostics[0].to_string().contains("linked.json"));

    // The root may be redirected, but helpers.d itself must be ordinary.
    let redirected_helpers = Directory::new();
    junction(
        &redirected_helpers.0.join("helpers.d"),
        &directory.helpers(),
    );
    let loaded = redirected_helpers.load().unwrap();
    assert!(loaded.external.is_empty());
    assert_eq!(loaded.diagnostics.len(), 1);
    assert!(loaded.diagnostics[0]
        .to_string()
        .contains("helper directory must be an ordinary directory"));
}

#[test]
fn capital_pitch_is_sparse_bounded_and_requires_registered_engine_ids() {
    use omnivox_core::settings::CapitalPitch;
    let config = parse_config(&json!({"schema":2,"speech":{"capital_pitch":{
        "default":1.4,"engines":{"espeak":"off","org.fixture":1.8}
    }}}))
    .unwrap();
    assert_eq!(config.speech.capital_pitch.pitch_for("winrt", 0.8), 1.4);
    assert_eq!(config.speech.capital_pitch.pitch_for("espeak", 0.8), 0.8);
    assert_eq!(
        config.speech.capital_pitch.pitch_for("org.fixture", 0.8),
        1.8
    );
    assert!(config.validate_overrides(&BTreeSet::new()).is_err());
    config
        .validate_overrides(&BTreeSet::from(["org.fixture".into()]))
        .unwrap();
    for value in [json!(0.5), json!(2), json!("off")] {
        parse_config(&json!({"schema":2,"speech":{"capital_pitch":{"default":value}}})).unwrap();
    }
    assert_eq!(
        parse_config(&json!({"schema":2,"speech":{"capital_pitch":{}}}))
            .unwrap()
            .speech
            .capital_pitch
            .default,
        CapitalPitch::Value(1.5)
    );
    for value in [
        json!(null),
        json!(false),
        json!(true),
        json!(0),
        json!(0.49),
        json!(2.01),
        json!(1e100),
        json!("1.5"),
        json!("OFF"),
        json!({}),
        json!([]),
    ] {
        for field in ["default", "engine"] {
            let cue = if field == "default" {
                json!({"default":value})
            } else {
                json!({"engines":{"espeak":value}})
            };
            assert!(
                parse_config(&json!({"schema":2,"speech":{"capital_pitch":cue}})).is_err(),
                "{field}: {value}"
            );
        }
    }
    for cue in [
        json!(null),
        json!({"extra":1}),
        json!({"engines":null}),
        json!({"engines":{"native":1.5}}),
        json!({"engines":{"bad id":1.5}}),
    ] {
        assert!(parse_config(&json!({"schema":2,"speech":{"capital_pitch":cue}})).is_err());
    }
    let engines: BTreeMap<_, _> = (0..=MAX_ENGINE_IDS)
        .map(|n| (format!("engine{n}"), 1.5))
        .collect();
    assert!(
        parse_config(&json!({"schema":2,"speech":{"capital_pitch":{"engines":engines}}})).is_err()
    );
    for bytes in [
        br#"{"schema":2,"speech":{"capital_pitch":{"default":1.4,"default":1.5}}}"#.as_slice(),
        br#"{"schema":2,"speech":{"capital_pitch":{"engines":{"espeak":1.4,"espeak":"off"}}}}"#,
        br#"{"schema":1,"speech":{"capital_pitch":{"default":1.5}}}"#,
    ] {
        assert!(Configuration::parse(bytes, Platform::native()).is_err());
    }
}

#[test]
fn audio_settings_are_sparse_strict_and_bounded() {
    use omnivox_core::settings::AudioBackend;
    use omnivox_core::state::ChannelMode;
    for audio in [
        json!({}),
        json!({"backend":"device"}),
        json!({"target":"both"}),
        json!({"pulse_latency_ms":20}),
    ] {
        assert_eq!(
            parse_config(&json!({"schema":2,"audio":audio}))
                .unwrap()
                .audio,
            AudioOutputSettings::default()
        );
    }
    let saved = parse_config(
        &json!({"schema":2,"audio":{"backend":"pulse","target":"right","pulse_latency_ms":45}}),
    )
    .unwrap()
    .audio;
    assert_eq!(saved.backend, AudioBackend::Pulse);
    assert_eq!(saved.target, ChannelMode::Right);
    assert_eq!(saved.pulse_latency_ms.get(), 45);
    for latency in [10, 200] {
        parse_config(&json!({"schema":2,"audio":{"pulse_latency_ms":latency}})).unwrap();
    }
    for audio in [
        json!(null),
        json!([]),
        json!({"backend":"Pulse"}),
        json!({"backend":"alsa"}),
        json!({"backend":null}),
        json!({"target":"LEFT"}),
        json!({"target":null}),
        json!({"target":false}),
        json!({"extra":true}),
        json!({"pulse_latency_ms":9}),
        json!({"pulse_latency_ms":201}),
        json!({"pulse_latency_ms":10.5}),
        json!({"pulse_latency_ms":"20"}),
        json!({"pulse_latency_ms":null}),
    ] {
        assert!(
            parse_config(&json!({"schema":2,"audio":audio})).is_err(),
            "{audio}"
        );
    }
    for bytes in [
        br#"{"schema":1,"audio":{}}"#.as_slice(),
        br#"{"schema":2,"audio":{"backend":"device","backend":"null"}}"#,
    ] {
        assert!(Configuration::parse(bytes, Platform::native()).is_err());
    }
}
