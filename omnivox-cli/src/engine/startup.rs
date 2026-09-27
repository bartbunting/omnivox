//! Capture and resolve all startup inputs before any native engine is created.
use super::*;
use omnivox_tts::engine_configuration::{
    EngineOrigin, LaunchSnapshot, ResolvedConfiguration, ResolvedRegistration, RuntimeInputs,
    RuntimeInvocation, Timeouts,
};
use std::collections::BTreeMap;
use std::ffi::OsStr;

pub(crate) struct EngineStartup {
    pub snapshot: LaunchSnapshot,
    pub library: Option<StartupLibrary>,
}

impl EngineStartup {
    pub fn read(
        engine: &str,
        model: Option<&str>,
        library: Option<&str>,
        config_dir: Option<&str>,
    ) -> Result<Self> {
        if let Some(snapshot) = crate::worker_startup::snapshot() {
            return Ok(Self::from_snapshot(snapshot.clone()));
        }
        Self::capture(
            engine,
            model,
            library,
            config_dir,
            LaunchEnvironment::capture(),
            &std::env::current_exe()?,
        )
    }

    pub fn from_snapshot(snapshot: LaunchSnapshot) -> Self {
        let library = snapshot.managed().map(StartupLibrary::from_frozen);
        Self { snapshot, library }
    }

    pub fn capture(
        engine: &str,
        model: Option<&str>,
        library: Option<&str>,
        config_dir: Option<&str>,
        environment: LaunchEnvironment,
        executable: &Path,
    ) -> Result<Self> {
        let root = Platform::native().configuration_root(config_dir.map(OsStr::new), |key| {
            environment.get(key).map(OsStr::to_owned)
        })?;
        let loaded = root.load()?;
        Self::from_inputs(engine, model, library, loaded, environment, executable)
    }

    pub(super) fn from_inputs(
        engine: &str,
        model: Option<&str>,
        library: Option<&str>,
        loaded: omnivox_tts::engine_configuration::LoadedConfiguration,
        environment: LaunchEnvironment,
        executable: &Path,
    ) -> Result<Self> {
        let library = StartupLibrary::from_captured_environment(library, model, &environment)?;
        let requested = if engine.is_empty() {
            environment
                .get("OMNIVOX_ENGINE")
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned()
        } else {
            engine.to_owned()
        };
        let configured_model = model
            .or_else(|| {
                environment
                    .get("OMNIVOX_PIPER_MODEL")
                    .and_then(OsStr::to_str)
            })
            .filter(|model| !model.is_empty());
        let piper_selected = requested == "piper"
            || configured_model.is_some()
            || loaded.configuration.engine_overrides.contains_key("piper")
            || routing_references(&loaded.configuration.routing, "piper")
            || library
                .as_ref()
                .is_some_and(|library| library.manages("piper"));
        let mut inputs = BTreeMap::<String, RuntimeInputs>::new();
        let piper = inputs.entry("piper".into()).or_default();
        piper.default_timeouts = Timeouts {
            startup_ms: Some(60_000),
            ..Timeouts::default()
        };
        if let Some(model) = configured_model {
            piper.invocation = Some(RuntimeInvocation::Legacy(vec![
                "--model".into(),
                model.into(),
            ]));
        }
        if !cfg!(feature = "piper") {
            piper.unavailable =
                Some("Piper support is not built in; rebuild Omnivox with --features piper".into());
        } else if configured_model.is_none()
            && !library
                .as_ref()
                .is_some_and(|library| library.manages("piper"))
            && !loaded
                .configuration
                .engine_overrides
                .get("piper")
                .is_some_and(|value| value.arguments.is_some())
        {
            piper.unavailable = Some("no Piper model is configured; use --piper-model, OMNIVOX_PIPER_MODEL or local arguments".into());
        }
        if !cfg!(any(windows, target_os = "linux")) {
            for id in ["eloquence", "dectalk"] {
                inputs.entry(id.into()).or_default().unavailable =
                    Some("engine is available only on Windows and Linux".into());
            }
        }
        if environment
            .get("OMNIVOX_MBROLA_HELPER")
            .filter(|value| !value.is_empty())
            .is_some_and(|path| !Path::new(path).is_absolute())
        {
            inputs.entry("mbrola".into()).or_default().unavailable =
                Some("OMNIVOX_MBROLA_HELPER must be an absolute prototype helper path".into());
        }
        if let Some(library) = &library {
            for id in ["piper", "flite", "mbrola", "rhvoice"] {
                let input = inputs.entry(id.into()).or_default();
                if library.manages(id) {
                    let mut config = HelperEngineConfig::with_environment(
                        id,
                        PathBuf::new(),
                        environment.clone(),
                    );
                    library.configure(&mut config);
                    input.invocation = Some(RuntimeInvocation::Managed(config.arguments));
                }
                if library.eligibility.excludes_provider(id) {
                    input.unavailable = Some("excluded by voice-library configuration".into());
                }
            }
        }
        let resolved = ResolvedConfiguration::resolve(
            loaded,
            executable,
            Platform::native(),
            environment,
            &inputs,
        )?;
        for diagnostic in &resolved.diagnostics {
            // Diagnostic CLI modes do not install a tracing subscriber.
            eprintln!("Engine configuration: {diagnostic}");
        }
        let snapshot = LaunchSnapshot::prepare(
            resolved,
            library.as_ref().map(StartupLibrary::freeze),
            requested,
            piper_selected,
        )?;
        Ok(Self { snapshot, library })
    }

    pub fn registry(&self) -> Result<EngineRegistry> {
        let resolved = self.snapshot.resolved();
        let permissions = resolved.selection_permissions();
        let mut registry = if let Some(library) = &self.library {
            library.registry_with_selection(resolved.routing.clone(), permissions)?
        } else {
            let mut registry = EngineRegistry::new();
            registry.configure_local_selection(resolved.routing.clone(), permissions)?;
            registry
        };
        for registration in resolved.registrations().filter(|entry| self.publish(entry)) {
            if registry
                .inventory()
                .iter()
                .any(|entry| entry.id == registration.engine_id)
            {
                continue;
            }
            if let Some(reason) = &registration.unavailable {
                let reason = reason.clone();
                registry.register_unavailable(
                    EngineDescriptor::unavailable(&registration.engine_id, &reason),
                    move || Err(reason.clone()),
                )?;
            }
        }
        Ok(registry)
    }

    fn publish(&self, entry: &ResolvedRegistration) -> bool {
        if entry.engine_id == "piper" && !self.snapshot.piper_selected() {
            return false;
        }
        entry.helper.is_some()
            || entry.source.is_some()
            || entry.override_source.is_some()
            || !entry.enabled
            || self.snapshot.requested() == entry.engine_id
            || routing_references(&self.snapshot.resolved().routing, &entry.engine_id)
            || self
                .library
                .as_ref()
                .is_some_and(|library| library.requires(&entry.engine_id))
    }

    pub fn helper_configs(&self, external: bool) -> Vec<HelperEngineConfig> {
        self.snapshot
            .resolved()
            .registrations()
            .filter(|entry| {
                entry.unavailable.is_none()
                    && self.publish(entry)
                    && (entry.origin == EngineOrigin::ExternalHelper) == external
            })
            .filter_map(|entry| entry.helper.clone())
            .collect()
    }

    pub fn permits_construction(&self, id: &str) -> bool {
        self.snapshot
            .resolved()
            .registration(id)
            .is_some_and(|entry| entry.unavailable.is_none())
    }

    pub fn order(&self) -> Vec<String> {
        self.snapshot.resolved().startup_order(
            self.snapshot.requested(),
            &engine_preference_order(self.snapshot.requested(), native_registry_engine_id()),
            native_registry_engine_id().unwrap_or("espeak"),
        )
    }

    pub fn external_priority(&self) -> &str {
        if self.snapshot.requested().is_empty() {
            self.snapshot
                .resolved()
                .routing
                .preferred_engine_ids
                .as_ref()
                .and_then(|ids| ids.first())
                .map(String::as_str)
                .unwrap_or("")
        } else {
            self.snapshot.requested()
        }
    }
}

fn routing_references(
    policy: &omnivox_tts::engine_configuration::LocalRoutingPolicy,
    id: &str,
) -> bool {
    policy
        .preferred_engine_ids
        .iter()
        .flatten()
        .chain(policy.fallback_engine_ids.iter().flatten())
        .chain(&policy.disabled_engine_ids)
        .chain(&policy.automatic_engine_ids)
        .any(|entry| entry == id)
}
