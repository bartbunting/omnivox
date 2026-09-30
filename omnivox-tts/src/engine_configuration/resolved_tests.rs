use super::*;
use crate::engine_configuration::{Configuration, HelperManifest, ManifestRegistration};

fn loaded(main: &str, manifests: &[&str]) -> LoadedConfiguration {
    LoadedConfiguration {
        root: Some(PathBuf::from("/configuration")),
        configuration: Configuration::parse(main.as_bytes(), Platform::Unix).unwrap(),
        external: manifests
            .iter()
            .map(|text| {
                let manifest = HelperManifest::parse(text.as_bytes(), Platform::Unix).unwrap();
                (
                    manifest.engine_id.clone(),
                    ManifestRegistration {
                        source: PathBuf::from(format!(
                            "/configuration/helpers.d/{}.json",
                            manifest.engine_id
                        )),
                        manifest,
                    },
                )
            })
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn resolve(
    loaded: LoadedConfiguration,
    vars: &[(&str, &str)],
    runtime: BTreeMap<String, RuntimeInputs>,
) -> Result<ResolvedConfiguration> {
    ResolvedConfiguration::resolve(
        loaded,
        Path::new("/nonexistent/omnivox"),
        Platform::Unix,
        LaunchEnvironment::from_variables(
            vars.iter().map(|(key, value)| (key.into(), value.into())),
        ),
        &runtime,
    )
}

#[test]
fn launch_fields_resolve_independently_and_arguments_stay_literal() {
    let input = loaded(
        r#"{
        "schema":1,
        "engine_overrides": {
            "org.test": {"program":"/override/with spaces/helper", "arguments":["", "$(no shell)", "${HOME}"], "timeouts":{"request_ms":999}},
            "flite": {"program":"/override/flite", "arguments":["local"], "timeouts":{"startup_ms":25000}}
        }
    }"#,
        &[
            r#"{"schema":1,"engine_id":"org.test","program":"/manifest/helper", "arguments":["discarded"],"timeouts":{"startup_ms":1234,"synthesis_idle_ms":55000}}"#,
        ],
    );
    let resolved = resolve(
        input,
        &[
            ("OMNIVOX_FLITE_HELPER", "/environment/flite"),
            ("OMNIVOX_ORG_TEST_HELPER", "/must-not-be-used"),
        ],
        BTreeMap::new(),
    )
    .unwrap();
    let external = resolved.registration("org.test").unwrap();
    assert_eq!(external.origin, EngineOrigin::ExternalHelper);
    assert_eq!(
        external.source.as_deref(),
        Some(Path::new("/configuration/helpers.d/org.test.json"))
    );
    assert_eq!(
        external.override_source.as_deref(),
        Some(Path::new("/configuration/config.json"))
    );
    let helper = external.helper.as_ref().unwrap();
    assert_eq!(helper.program, Path::new("/override/with spaces/helper"));
    assert_eq!(
        helper.arguments,
        ["", "$(no shell)", "${HOME}"].map(OsString::from)
    );
    assert_eq!(helper.startup_timeout, Duration::from_millis(1234));
    assert_eq!(helper.request_timeout, Duration::from_millis(999));
    assert_eq!(helper.synthesis_idle_timeout, Duration::from_millis(55000));
    assert_eq!(helper.environment, resolved.environment);
    let shipped = resolved
        .registration("flite")
        .unwrap()
        .helper
        .as_ref()
        .unwrap();
    assert_eq!(shipped.program, Path::new("/environment/flite"));
    assert_eq!(shipped.arguments, [OsString::from("local")]);
    assert_eq!(shipped.startup_timeout, Duration::from_secs(25));
    assert_eq!(shipped.synthesis_idle_timeout, Duration::from_secs(60));
}

#[test]
fn empty_legacy_environment_uses_the_local_program_and_shipped_defaults() {
    let input = loaded(
        r#"{"schema":1,"engine_overrides":{"eloquence":{"program":"/private/eci"}}}"#,
        &[],
    );
    let resolved = resolve(input, &[("OMNIVOX_ELOQUENCE_HELPER", "")], BTreeMap::new()).unwrap();
    let helper = resolved
        .registration("eloquence")
        .unwrap()
        .helper
        .as_ref()
        .unwrap();
    assert_eq!(helper.program, Path::new("/private/eci"));
    assert_eq!(helper.synthesis_idle_timeout, Duration::from_millis(500));
    assert!(resolved
        .registration("macos")
        .unwrap()
        .unavailable
        .is_some());
}

#[test]
fn managed_invocations_reject_argument_replacement_even_when_disabled() {
    let mut runtime = BTreeMap::from([(
        "piper".into(),
        RuntimeInputs {
            invocation: Some(RuntimeInvocation::Managed(vec![
                "--voice-library".into(),
                "/generation".into(),
            ])),
            default_timeouts: Timeouts {
                startup_ms: Some(60000),
                ..Timeouts::default()
            },
            unavailable: None,
        },
    )]);
    let input = loaded(
        r#"{"schema":1,"engine_overrides":{"piper":{"arguments":["private argument"],"enabled":false}}}"#,
        &[],
    );
    let error = resolve(input, &[], runtime.clone()).err().unwrap();
    assert!(error
        .field
        .contains("config.json: engine_overrides.piper.arguments"));
    assert!(!error.to_string().contains("private argument"));
    let input = loaded(
        r#"{"schema":1,"engine_overrides":{"piper":{"program":"/piper", "timeouts":{"request_ms":456}}}}"#,
        &[],
    );
    let resolved = resolve(input, &[], runtime.clone()).unwrap();
    let helper = resolved
        .registration("piper")
        .unwrap()
        .helper
        .as_ref()
        .unwrap();
    assert_eq!(
        helper.arguments,
        ["--voice-library", "/generation"].map(OsString::from)
    );
    assert_eq!(helper.startup_timeout, Duration::from_secs(60));
    assert_eq!(helper.request_timeout, Duration::from_millis(456));

    runtime.get_mut("piper").unwrap().invocation = Some(RuntimeInvocation::Legacy(vec![
        "--model".into(),
        "explicit model.onnx".into(),
    ]));
    let input = loaded(
        r#"{"schema":1,"engine_overrides":{"piper":{"program":"/piper", "arguments":["local"]}}}"#,
        &[],
    );
    let resolved = resolve(input, &[], runtime).unwrap();
    assert_eq!(
        resolved
            .registration("piper")
            .unwrap()
            .helper
            .as_ref()
            .unwrap()
            .arguments,
        ["--model", "explicit model.onnx"].map(OsString::from)
    );
}

#[test]
fn disablement_and_external_permission_remain_independent_of_preference() {
    let input = loaded(
        r#"{"schema":1,"routing":{"preferred_engine_ids":["org.disabled", "org.explicit"], "disabled_engine_ids":["flite"], "automatic_engine_ids":["org.disabled", "org.automatic"]}, "engine_overrides":{"espeak":{"enabled":false}}}"#,
        &[
            r#"{"schema":1,"engine_id":"org.disabled","program":"/helper","enabled":false}"#,
            r#"{"schema":1,"engine_id":"org.explicit","program":"/helper"}"#,
            r#"{"schema":1,"engine_id":"org.automatic","program":"/helper"}"#,
        ],
    );
    let resolved = resolve(input, &[], BTreeMap::new()).unwrap();
    let permissions = resolved.selection_permissions();
    assert!(permissions.disabled("espeak"));
    assert!(permissions.disabled("flite"));
    assert!(permissions.disabled("org.disabled"));
    assert!(!permissions.permits_automatic("org.explicit"));
    assert!(permissions.permits_automatic("org.automatic"));
    assert!(permissions.permits_automatic("rhvoice"));
    let order = resolved.startup_order("org.startup", &["espeak", "flite"], "espeak");
    assert_eq!(
        order,
        [
            "org.startup",
            "org.disabled",
            "org.explicit",
            "espeak",
            "flite"
        ]
    );
    assert!(!order.contains(&"org.automatic".into()));
    for id in ["espeak", "flite", "org.disabled"] {
        let registration = resolved.registration(id).unwrap();
        assert!(!registration.enabled);
        assert_eq!(
            registration.unavailable.as_deref(),
            Some("disabled by local engine configuration")
        );
    }
}

#[test]
fn enabling_a_manifest_preserves_routing_and_runtime_exclusions() {
    let input = loaded(
        r#"{"schema":1,"routing":{"disabled_engine_ids":["org.excluded"]},"engine_overrides":{"org.enabled":{"enabled":true},"org.excluded":{"enabled":true},"piper":{"enabled":true}}}"#,
        &[
            r#"{"schema":1,"engine_id":"org.enabled","program":"/helper","enabled":false}"#,
            r#"{"schema":1,"engine_id":"org.excluded","program":"/helper","enabled":false}"#,
        ],
    );
    let runtime = BTreeMap::from([(
        "piper".into(),
        RuntimeInputs {
            unavailable: Some("managed assets failed validation".into()),
            ..RuntimeInputs::default()
        },
    )]);
    let resolved = resolve(input, &[], runtime).unwrap();
    assert!(resolved.registration("org.enabled").unwrap().enabled);
    assert!(!resolved.registration("org.excluded").unwrap().enabled);
    assert!(resolved.selection_permissions().disabled("org.excluded"));
    assert_eq!(
        resolved
            .registration("piper")
            .unwrap()
            .unavailable
            .as_deref(),
        Some("managed assets failed validation")
    );
}

#[test]
fn empty_preference_preserves_default_selection_without_populating_policy() {
    let input = loaded(
        r#"{"schema":1,"routing":{"preferred_engine_ids":[], "fallback_engine_ids":[],"automatic_engine_ids":["not.installed"]}}"#,
        &[],
    );
    let resolved = resolve(input, &[], BTreeMap::new()).unwrap();
    assert_eq!(
        resolved.startup_order("", &["espeak", "flite"], "espeak"),
        ["espeak", "flite"]
    );
    assert_eq!(resolved.routing.preferred_engine_ids, Some(vec![]));
    assert_eq!(resolved.routing.fallback_engine_ids, Some(vec![]));
    assert_eq!(resolved.diagnostics.len(), 1);
    assert!(resolved.registration("not.installed").is_none());
    assert_eq!(
        resolved.startup_order("native", &["espeak", "flite"], "espeak"),
        ["espeak", "flite"]
    );
}
