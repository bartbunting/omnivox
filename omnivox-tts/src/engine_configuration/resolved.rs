//! Resolve private launch data once, before native construction. Provider and
//! compiled-feature inputs come from trusted host code, never from manifests.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::helper_engine::HelperEngineConfig;

use super::{
    shipped, ConfigurationError, EngineOverride, EngineSelectionPermissions, LaunchEnvironment,
    LoadedConfiguration, LocalRoutingPolicy, Platform, Result, Timeouts,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineOrigin {
    InProcess,
    ShippedHelper,
    ExternalHelper,
}

/// A provider owns a complete invocation. Explicit legacy runtime options
/// retain their existing precedence; managed load arguments reject replacement.
#[derive(Clone)]
pub enum RuntimeInvocation {
    Legacy(Vec<OsString>),
    Managed(Vec<OsString>),
}

/// Trusted adapter inputs resolved using the same captured environment. For
/// example, Piper has qualified startup defaults and compiled feature gating.
#[derive(Clone, Default)]
pub struct RuntimeInputs {
    pub invocation: Option<RuntimeInvocation>,
    pub default_timeouts: Timeouts,
    pub unavailable: Option<String>,
}

/// No Debug implementation: the launch record contains private arguments.
#[derive(Clone)]
pub struct ResolvedRegistration {
    pub engine_id: String,
    pub origin: EngineOrigin,
    pub source: Option<PathBuf>,
    pub override_source: Option<PathBuf>,
    pub enabled: bool,
    pub unavailable: Option<String>,
    pub helper: Option<HelperEngineConfig>,
}

/// One complete registration set. Building it reads installation-relative
/// executable metadata but never constructs an engine or launches a process.
#[derive(Clone)]
pub struct ResolvedConfiguration {
    pub(super) registrations: BTreeMap<String, ResolvedRegistration>,
    pub routing: LocalRoutingPolicy,
    pub speech: super::SpeechConfiguration,
    pub environment: LaunchEnvironment,
    pub root: Option<PathBuf>,
    pub diagnostics: Vec<ConfigurationError>,
}

impl ResolvedConfiguration {
    pub fn resolve(
        loaded: LoadedConfiguration,
        executable: &Path,
        platform: Platform,
        environment: LaunchEnvironment,
        runtime: &BTreeMap<String, RuntimeInputs>,
    ) -> Result<Self> {
        let external_ids = loaded.external.keys().cloned().collect();
        loaded.configuration.validate_overrides(&external_ids)?;
        let mut registrations = BTreeMap::new();
        let main_source = loaded.root.as_ref().map(|root| root.join("config.json"));
        for definition in shipped::ENGINES {
            let id = definition.id;
            let override_value = loaded.configuration.engine_overrides.get(id);
            let inputs = runtime.get(id).cloned().unwrap_or_default();
            if matches!(inputs.invocation, Some(RuntimeInvocation::Managed(_)))
                && override_value.is_some_and(|value| value.arguments.is_some())
            {
                return Err(ConfigurationError::new(
                    format!(
                        "{}: engine_overrides.{id}.arguments",
                        main_source
                            .as_deref()
                            .unwrap_or_else(|| Path::new("config.json"))
                            .display()
                    ),
                    "arguments are owned by managed voice-library configuration",
                ));
            }
            let helper = if definition.in_process {
                None
            } else {
                let environment_program = definition
                    .helper_environment
                    .and_then(|key| environment.get(key))
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from);
                let program = environment_program
                    .or_else(|| {
                        override_value
                            .and_then(|value| value.program.as_ref())
                            .map(PathBuf::from)
                    })
                    .or_else(|| {
                        shipped::resolve_adjacent(
                            executable,
                            &definition.helper_candidates(platform),
                        )
                    });
                program.map(|program| {
                    let mut helper =
                        HelperEngineConfig::with_environment(id, program, environment.clone());
                    helper.synthesis_idle_timeout = definition.synthesis_idle_timeout();
                    apply_timeouts(&mut helper, &inputs.default_timeouts);
                    apply_override(&mut helper, override_value);
                    if let Some(invocation) = &inputs.invocation {
                        helper.arguments = match invocation {
                            RuntimeInvocation::Legacy(arguments)
                            | RuntimeInvocation::Managed(arguments) => arguments.clone(),
                        };
                    }
                    helper
                })
            };
            let enabled = override_value
                .and_then(|value| value.enabled)
                .unwrap_or(true)
                && !loaded
                    .configuration
                    .routing
                    .disabled_engine_ids
                    .iter()
                    .any(|disabled| disabled == id);
            let unavailable = if !enabled {
                Some("disabled by local engine configuration".into())
            } else if let Some(reason) = inputs.unavailable {
                Some(reason)
            } else if definition.in_process {
                match (id, platform) {
                    ("espeak", _) | ("winrt", Platform::Windows) | ("macos", Platform::MacOs) => {
                        None
                    }
                    _ => Some("engine is not available on this platform".into()),
                }
            } else if helper.is_none() {
                Some("helper executable is not configured or installed".into())
            } else {
                None
            };
            registrations.insert(
                id.into(),
                ResolvedRegistration {
                    engine_id: id.into(),
                    origin: if definition.in_process {
                        EngineOrigin::InProcess
                    } else {
                        EngineOrigin::ShippedHelper
                    },
                    source: None,
                    override_source: override_value.and(main_source.clone()),
                    enabled,
                    unavailable,
                    helper,
                },
            );
        }
        for (id, registration) in loaded.external {
            let manifest = registration.manifest;
            let override_value = loaded.configuration.engine_overrides.get(&id);
            let mut helper = HelperEngineConfig::with_environment(
                &id,
                override_value
                    .and_then(|value| value.program.as_ref())
                    .unwrap_or(&manifest.program),
                environment.clone(),
            );
            helper.arguments = manifest.arguments.into_iter().map(OsString::from).collect();
            helper.synthesis_idle_timeout = Duration::from_secs(60);
            apply_timeouts(&mut helper, &manifest.timeouts);
            apply_override(&mut helper, override_value);
            let enabled = override_value
                .and_then(|value| value.enabled)
                .unwrap_or(manifest.enabled)
                && !loaded
                    .configuration
                    .routing
                    .disabled_engine_ids
                    .contains(&id);
            registrations.insert(
                id.clone(),
                ResolvedRegistration {
                    engine_id: id,
                    origin: EngineOrigin::ExternalHelper,
                    source: Some(registration.source),
                    override_source: override_value.and(main_source.clone()),
                    enabled,
                    unavailable: (!enabled)
                        .then(|| "disabled by local engine configuration".into()),
                    helper: Some(helper),
                },
            );
        }
        let mut diagnostics = loaded.diagnostics;
        let policy = &loaded.configuration.routing;
        let referenced: BTreeSet<_> = policy
            .preferred_engine_ids
            .iter()
            .flatten()
            .chain(policy.fallback_engine_ids.iter().flatten())
            .chain(&policy.disabled_engine_ids)
            .chain(&policy.automatic_engine_ids)
            .collect();
        for id in referenced {
            if !registrations.contains_key(id) {
                diagnostics.push(ConfigurationError::new(
                    format!("routing.{id}"),
                    "referenced engine is not registered",
                ));
            }
        }
        Ok(Self {
            registrations,
            routing: loaded.configuration.routing,
            speech: loaded.configuration.speech,
            root: loaded.root,
            diagnostics,
            environment,
        })
    }

    pub fn registrations(&self) -> impl Iterator<Item = &ResolvedRegistration> {
        self.registrations.values()
    }

    pub fn registration(&self, id: &str) -> Option<&ResolvedRegistration> {
        self.registrations.get(id)
    }

    pub fn selection_permissions(&self) -> EngineSelectionPermissions {
        EngineSelectionPermissions::new(
            self.registrations
                .values()
                .filter(|entry| !entry.enabled)
                .map(|entry| entry.engine_id.clone())
                .chain(self.routing.disabled_engine_ids.iter().cloned())
                .collect(),
            self.registrations
                .values()
                .filter(|entry| entry.origin == EngineOrigin::ExternalHelper)
                .map(|entry| entry.engine_id.clone())
                .collect(),
            &self.routing.automatic_engine_ids.iter().cloned().collect(),
        )
    }

    /// Explicit targets precede the local preference list and the established
    /// platform order. Automatic permission never appends external candidates.
    pub fn startup_order(
        &self,
        requested: &str,
        platform_defaults: &[&str],
        native: &str,
    ) -> Vec<String> {
        let mut order = Vec::new();
        for id in (!requested.is_empty())
            .then_some(requested)
            .into_iter()
            .chain(
                self.routing
                    .preferred_engine_ids
                    .iter()
                    .flatten()
                    .map(String::as_str),
            )
            .chain(platform_defaults.iter().copied())
        {
            let id = if id == "native" { native } else { id };
            if !order.iter().any(|entry| entry == id) {
                order.push(id.to_owned());
            }
        }
        order
    }
}

fn apply_override(helper: &mut HelperEngineConfig, value: Option<&EngineOverride>) {
    if let Some(value) = value {
        if let Some(arguments) = &value.arguments {
            helper.arguments = arguments.iter().map(OsString::from).collect();
        }
        if let Some(timeouts) = &value.timeouts {
            apply_timeouts(helper, timeouts);
        }
    }
}

fn apply_timeouts(helper: &mut HelperEngineConfig, value: &Timeouts) {
    if let Some(ms) = value.startup_ms {
        helper.startup_timeout = Duration::from_millis(ms);
    }
    if let Some(ms) = value.request_ms {
        helper.request_timeout = Duration::from_millis(ms);
    }
    if let Some(ms) = value.synthesis_idle_ms {
        helper.synthesis_idle_timeout = Duration::from_millis(ms);
    }
}

#[cfg(test)]
#[path = "resolved_tests.rs"]
mod tests;
