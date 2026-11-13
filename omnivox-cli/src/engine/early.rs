//! Private early-speech experiment. Ordinary and managed startup stay eager.
use super::*;

pub(super) fn partition(
    startup: &EngineStartup,
) -> (Vec<HelperEngineConfig>, Vec<HelperEngineConfig>) {
    let configs = startup.helper_configs(false);
    let selected = startup.order().into_iter().next().unwrap_or_default();
    let enabled = startup
        .snapshot
        .resolved()
        .environment
        .get("OMNIVOX_EXPERIMENTAL_EARLY_SPEECH")
        == Some(std::ffi::OsStr::new("1"))
        && crate::worker_startup::snapshot().is_some()
        && std::env::var_os("OMNIVOX_REMOTE_WORKER").is_none()
        && startup.library.is_none()
        && startup.helper_configs(true).is_empty()
        && selected != "tgspeechbox";
    if !enabled {
        return (configs, Vec::new());
    }
    configs
        .into_iter()
        .partition(|config| config.engine_id == selected)
}

pub(super) fn discover(
    registry: &mut EngineRegistry,
    configs: Vec<HelperEngineConfig>,
    generation: Arc<AtomicU64>,
    budget: Arc<IsolationBudget>,
) -> Result<()> {
    for config in configs {
        let id = config.engine_id.clone();
        // Retain the same native owner through failure and explicit retry,
        // matching ordinary failed-startup recovery. Never replace uncertain
        // ownership with a fresh engine object.
        let owner = Mutex::new(None::<Arc<HelperTtsEngine>>);
        let generation = generation.clone();
        let budget = budget.clone();
        registry.register_unavailable(
            EngineDescriptor::unavailable(&id, "Optional engine discovery pending"),
            move || {
                let engine = {
                    let mut owner = owner.lock().map_err(|_| "helper owner lock poisoned")?;
                    if owner.is_none() {
                        *owner = Some(Arc::new(
                            HelperTtsEngine::prepare(config.clone())
                                .map_err(|error| error.to_string())?,
                        ));
                    }
                    owner.as_ref().unwrap().clone()
                };
                engine
                    .refresh_connection()
                    .map_err(|error| error.to_string())?;
                Ok(Arc::new(IsolatedTtsEngine::new(
                    engine,
                    generation.clone(),
                    budget.clone(),
                )) as Arc<dyn TtsEngine>)
            },
        )?;
        // Existing per-engine admission bounds concurrent discovery and rescan;
        // success publishes the validated descriptor and handle atomically.
        if let Err(error) = registry.request_rescan(&id) {
            warn!(engine_id = id, %error, "Optional engine discovery could not start");
        }
    }
    Ok(())
}
