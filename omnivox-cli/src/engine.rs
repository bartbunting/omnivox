//! TTS engine selection and creation.

use anyhow::Result;
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::espeak::EspeakTtsEngine;
use omnivox_tts::helper_engine::{
    load_helper_descriptor_cache, HelperEngineConfig, HelperTtsEngine,
    HELPER_DESCRIPTOR_CACHE_FILE_NAME,
};
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::TtsEngine;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};

const ELOQUENCE_SYNTHESIS_IDLE_TIMEOUT: Duration = Duration::from_millis(500);
const NATIVE_HELPER_SYNTHESIS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TGSPEECHBOX_SAMPLE_RATE_ENVIRONMENT_VARIABLE: &str = "OMNIVOX_TGSPEECHBOX_SAMPLE_RATE";
const TGSPEECHBOX_22050_CACHE_FILE_NAME: &str = "VOICE-INVENTORY-22050.json";
const TGSPEECHBOX_44100_CACHE_FILE_NAME: &str = "VOICE-INVENTORY-44100.json";

fn helper_synthesis_idle_timeout(engine_id: &str) -> Duration {
    if engine_id == "eloquence" {
        ELOQUENCE_SYNTHESIS_IDLE_TIMEOUT
    } else {
        NATIVE_HELPER_SYNTHESIS_IDLE_TIMEOUT
    }
}

/// Engines initialized for one server session.
pub struct CreatedEngines {
    pub preferred: Arc<dyn TtsEngine>,
    pub registry: EngineRegistry,
}

/// Create all engines that should be available to the server process.
///
/// Server mode initializes available engines so the first inventory is complete
/// and runtime routing can retain fallbacks. Independent helper processes
/// initialize concurrently with the built-in engines. TGSpeechBox may register
/// from its build-time inventory cache and defer its process until first use.
/// Piper remains opt-in through model configuration because starting its helper
/// loads a comparatively large voice model.
pub fn create_engines(
    engine_name: &str,
    piper_model: Option<&str>,
    generation: Arc<AtomicU64>,
) -> Result<CreatedEngines> {
    let isolation_budget = Arc::new(IsolationBudget::new());
    #[cfg(target_os = "windows")]
    {
        create_windows_engines(engine_name, piper_model, generation, isolation_budget)
    }

    #[cfg(not(target_os = "windows"))]
    {
        create_non_windows_engines(engine_name, piper_model, generation, isolation_budget)
    }
}

#[cfg(not(target_os = "windows"))]
fn create_non_windows_engines(
    engine_name: &str,
    piper_model: Option<&str>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<CreatedEngines> {
    let requested = requested_engine(engine_name);
    let helper_initializations =
        start_helper_initializations(configured_helper_configs(&requested, piper_model));
    let mut registry = EngineRegistry::new();

    #[cfg(target_os = "macos")]
    match MacOsTtsEngine::new() {
        Ok(engine) => {
            let engine: Arc<dyn TtsEngine> = Arc::new(engine);
            registry.register(isolate_server_engine(
                engine,
                Arc::clone(&generation),
                Arc::clone(&isolation_budget),
            ))?;
            info!("Registered macOS AVSpeechSynthesizer engine");
        }
        Err(error) => warn!("macOS AVSpeechSynthesizer not available: {error}"),
    }

    match EspeakTtsEngine::new() {
        Ok(engine) => {
            let engine: Arc<dyn TtsEngine> = Arc::new(engine);
            registry.register(isolate_server_engine(
                engine,
                Arc::clone(&generation),
                Arc::clone(&isolation_budget),
            ))?;
            info!("Registered espeak-ng fallback engine");
        }
        Err(error) => warn!("espeak-ng fallback not available: {error}"),
    }

    register_initialized_helpers(
        &mut registry,
        helper_initializations,
        generation,
        isolation_budget,
    )?;

    let preferred = engine_preference_order(&requested, native_registry_engine_id())
        .iter()
        .find_map(|engine_id| registry.engine(engine_id))
        .ok_or_else(|| anyhow::anyhow!("No TTS engine available"))?;
    info!(
        "Using {} as the preferred TTS engine",
        preferred.descriptor().id
    );

    Ok(CreatedEngines {
        preferred,
        registry,
    })
}

#[cfg(target_os = "windows")]
fn create_windows_engines(
    engine_name: &str,
    piper_model: Option<&str>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<CreatedEngines> {
    let forced = requested_engine(engine_name);
    let mut helper_configs = [
        helper_config(
            "eloquence",
            "OMNIVOX_ELOQUENCE_HELPER",
            "OmnivoxEloquenceHelper32.exe",
        ),
        helper_config(
            "dectalk",
            "OMNIVOX_DECTALK_HELPER",
            "OmnivoxDectalkHelper32.exe",
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    helper_configs.extend(configured_helper_configs(&forced, piper_model));
    let helper_initializations = start_helper_initializations(helper_configs);
    let mut registry = EngineRegistry::new();

    match WindowsTtsEngine::new() {
        Ok(engine) => {
            let engine: Arc<dyn TtsEngine> = Arc::new(engine);
            registry.register(Arc::new(IsolatedTtsEngine::new(
                engine,
                Arc::clone(&generation),
                Arc::clone(&isolation_budget),
            )))?;
            info!("Registered Windows WinRT engine");
        }
        Err(error) => warn!("Windows WinRT not available: {}", error),
    }

    match EspeakTtsEngine::new() {
        Ok(engine) => {
            registry.register(Arc::new(engine))?;
            info!("Registered espeak-ng fallback engine");
        }
        Err(error) => warn!("espeak-ng fallback not available: {}", error),
    }

    register_initialized_helpers(
        &mut registry,
        helper_initializations,
        generation,
        isolation_budget,
    )?;

    let preferred = engine_preference_order(&forced, Some("winrt"))
        .iter()
        .find_map(|engine_id| registry.engine(engine_id))
        .ok_or_else(|| anyhow::anyhow!("No TTS engine available"))?;
    info!(
        "Using {} as the preferred TTS engine",
        preferred.descriptor().id
    );

    Ok(CreatedEngines {
        preferred,
        registry,
    })
}

#[cfg(target_os = "windows")]
fn helper_config(
    engine_id: &str,
    environment_variable: &str,
    adjacent_filename: &str,
) -> Option<HelperEngineConfig> {
    helper_config_with_candidates(
        engine_id,
        environment_variable,
        &[PathBuf::from(adjacent_filename)],
    )
}

fn helper_config_with_candidates(
    engine_id: &str,
    environment_variable: &str,
    adjacent_candidates: &[PathBuf],
) -> Option<HelperEngineConfig> {
    let explicitly_configured = std::env::var_os(environment_variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let program = match explicitly_configured {
        Some(program) => program,
        None => resolve_adjacent_helper(&std::env::current_exe().ok()?, adjacent_candidates)?,
    };

    let mut config = HelperEngineConfig::new(engine_id, program);
    // Version-1 native helpers capture one complete utterance before emitting
    // PCM. Eloquence normally returns in milliseconds, so fail over promptly
    // when its native eciSynchronize call wedges. Other helpers retain the
    // longer allowance needed by ordinary long passages.
    config.synthesis_idle_timeout = helper_synthesis_idle_timeout(engine_id);
    Some(config)
}

fn resolve_adjacent_helper(executable: &Path, candidates: &[PathBuf]) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    candidates
        .iter()
        .map(|candidate| executable_dir.join(candidate))
        .find(|candidate| candidate.is_file())
}

struct PendingHelperInitialization<T> {
    engine_id: String,
    helper_path: PathBuf,
    config: HelperEngineConfig,
    handle: std::io::Result<JoinHandle<(T, Duration)>>,
}

type HelperConstructionResult =
    Result<HelperTtsEngine, omnivox_tts::helper_engine::HelperEngineError>;
type HelperInitializationResult =
    Result<Arc<HelperTtsEngine>, omnivox_tts::helper_engine::HelperEngineError>;
type PendingHelper = PendingHelperInitialization<HelperInitializationResult>;

fn start_helper_initializations(configs: Vec<HelperEngineConfig>) -> Vec<PendingHelper> {
    start_helper_initializations_with(configs, |config| {
        let engine_id = config.engine_id.clone();
        let helper_path = config.program.clone();
        let result = initialize_server_helper(config).map(Arc::new);
        if engine_id == "tgspeechbox" {
            if let Ok(engine) = &result {
                spawn_tgspeechbox_prewarm(Arc::clone(engine), helper_path);
            }
        }
        result
    })
}

fn initialize_server_helper(config: HelperEngineConfig) -> HelperConstructionResult {
    if config.engine_id != "tgspeechbox" {
        return HelperTtsEngine::new(config);
    }

    let helper_directory = config.program.parent().unwrap_or_else(|| Path::new(""));
    let cache_file_name = tgspeechbox_descriptor_cache_file_name(
        std::env::var_os(TGSPEECHBOX_SAMPLE_RATE_ENVIRONMENT_VARIABLE).as_deref(),
    );
    let mut cache_path = helper_directory.join(cache_file_name);
    if cache_file_name == TGSPEECHBOX_44100_CACHE_FILE_NAME && !cache_path.is_file() {
        cache_path = helper_directory.join(HELPER_DESCRIPTOR_CACHE_FILE_NAME);
    }
    match load_helper_descriptor_cache(&cache_path, &config.engine_id) {
        Ok(descriptor) => {
            info!(
                engine_id = config.engine_id,
                helper = %config.program.display(),
                cache = %cache_path.display(),
                "Prepared deferred helper from cached voice inventory"
            );
            HelperTtsEngine::new_deferred(config, descriptor)
        }
        Err(error) => {
            warn!(
                engine_id = config.engine_id,
                helper = %config.program.display(),
                cache = %cache_path.display(),
                %error,
                "Cached voice inventory is unavailable; initializing helper eagerly"
            );
            HelperTtsEngine::new(config)
        }
    }
}

fn tgspeechbox_descriptor_cache_file_name(sample_rate: Option<&std::ffi::OsStr>) -> &'static str {
    if sample_rate == Some(std::ffi::OsStr::new("22050")) {
        TGSPEECHBOX_22050_CACHE_FILE_NAME
    } else {
        TGSPEECHBOX_44100_CACHE_FILE_NAME
    }
}

fn start_helper_initializations_with<T, F>(
    configs: Vec<HelperEngineConfig>,
    initialize: F,
) -> Vec<PendingHelperInitialization<T>>
where
    T: Send + 'static,
    F: Fn(HelperEngineConfig) -> T + Send + Sync + 'static,
{
    let initialize = Arc::new(initialize);
    configs
        .into_iter()
        .map(|config| {
            let engine_id = config.engine_id.clone();
            let helper_path = config.program.clone();
            let thread_name = format!("omnivox-{engine_id}-init");
            let initialize = Arc::clone(&initialize);
            let thread_config = config.clone();
            let handle = thread::Builder::new().name(thread_name).spawn(move || {
                let started_at = Instant::now();
                let result = initialize(thread_config);
                (result, started_at.elapsed())
            });
            PendingHelperInitialization {
                engine_id,
                helper_path,
                config,
                handle,
            }
        })
        .collect()
}

fn register_initialized_helpers(
    registry: &mut EngineRegistry,
    pending: Vec<PendingHelper>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<()> {
    for initialization in pending {
        let PendingHelperInitialization {
            engine_id,
            helper_path,
            config,
            handle,
        } = initialization;
        let result = handle
            .map_err(|error| format!("Could not start helper initialization: {error}"))
            .and_then(|handle| {
                handle
                    .join()
                    .map_err(|_| "Helper initialization thread panicked".to_owned())
            })
            .and_then(|(result, elapsed)| {
                result
                    .map(|engine| (engine, elapsed))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok((engine, elapsed)) => {
                let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
                let registered_engine: Arc<dyn TtsEngine> = engine;
                registry.register(Arc::new(IsolatedTtsEngine::new(
                    registered_engine,
                    Arc::clone(&generation),
                    Arc::clone(&isolation_budget),
                )))?;
                info!(engine_id, helper = %helper_path.display(), elapsed_ms, "Registered helper engine");
            }
            Err(reason) => {
                warn!(engine_id, helper = %helper_path.display(), %reason, "Helper is not available");
                let generation = Arc::clone(&generation);
                let isolation_budget = Arc::clone(&isolation_budget);
                registry.register_unavailable(
                    EngineDescriptor::unavailable(&engine_id, reason),
                    move || {
                        // A rescan must perform live discovery, including for
                        // helpers that normally permit a deferred cached inventory.
                        let engine = HelperTtsEngine::new(config.clone())
                            .map_err(|error| error.to_string())?;
                        Ok(Arc::new(IsolatedTtsEngine::new(
                            Arc::new(engine),
                            Arc::clone(&generation),
                            Arc::clone(&isolation_budget),
                        )) as Arc<dyn TtsEngine>)
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn spawn_tgspeechbox_prewarm(engine: Arc<HelperTtsEngine>, helper_path: PathBuf) {
    let thread_name = "omnivox-tgspeechbox-prewarm";
    let thread_helper_path = helper_path.clone();
    let spawn = thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            let started_at = Instant::now();
            match engine.prewarm_connection() {
                Ok(true) => info!(
                    engine_id = "tgspeechbox",
                    helper = %thread_helper_path.display(),
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "Pre-warmed helper engine"
                ),
                Ok(false) => {}
                Err(error) => warn!(
                    engine_id = "tgspeechbox",
                    helper = %thread_helper_path.display(),
                    elapsed_ms = started_at.elapsed().as_millis(),
                    %error,
                    "Could not pre-warm helper engine; first synthesis will retry"
                ),
            }
        });
    if let Err(error) = spawn {
        warn!(
            engine_id = "tgspeechbox",
            helper = %helper_path.display(),
            %error,
            "Could not start helper pre-warm thread"
        );
    }
}

fn companion_helper_config(
    engine_id: &str,
    environment_variable: &str,
) -> Option<HelperEngineConfig> {
    let candidates = companion_helper_candidates(engine_id);
    helper_config_with_candidates(engine_id, environment_variable, &candidates)
}

fn companion_helper_candidates(engine_id: &str) -> [PathBuf; 2] {
    let helper_filename = format!("omnivox-{engine_id}-helper{}", std::env::consts::EXE_SUFFIX);
    [
        PathBuf::from(engine_id).join(&helper_filename),
        PathBuf::from(&helper_filename),
    ]
}

fn companion_helper_configs() -> Vec<HelperEngineConfig> {
    [
        ("rhvoice", "OMNIVOX_RHVOICE_HELPER"),
        ("flite", "OMNIVOX_FLITE_HELPER"),
        ("rutts", "OMNIVOX_RUTTS_HELPER"),
        ("tgspeechbox", "OMNIVOX_TGSPEECHBOX_HELPER"),
    ]
    .into_iter()
    .filter_map(|(engine_id, environment_variable)| {
        companion_helper_config(engine_id, environment_variable)
    })
    .collect()
}

fn requested_engine(engine_name: &str) -> String {
    if engine_name.is_empty() {
        std::env::var("OMNIVOX_ENGINE").unwrap_or_default()
    } else {
        engine_name.to_owned()
    }
}

fn engine_preference_order(
    requested: &str,
    native_engine_id: Option<&'static str>,
) -> Vec<&'static str> {
    let mut order = Vec::with_capacity(9);
    match requested {
        "espeak" => order.push("espeak"),
        "piper" => order.push("piper"),
        "rhvoice" => order.push("rhvoice"),
        "flite" => order.push("flite"),
        "rutts" => order.push("rutts"),
        "tgspeechbox" => order.push("tgspeechbox"),
        "eloquence" => order.push("eloquence"),
        "dectalk" => order.push("dectalk"),
        _ => {
            if let Some(native) = native_engine_id {
                order.push(native);
            }
        }
    }
    if !order.contains(&"espeak") {
        order.push("espeak");
    }
    if let Some(native) = native_engine_id {
        if !order.contains(&native) {
            order.push(native);
        }
    }
    if native_engine_id == Some("winrt") {
        if !order.contains(&"eloquence") {
            order.push("eloquence");
        }
        if !order.contains(&"dectalk") {
            order.push("dectalk");
        }
    }
    if !order.contains(&"piper") {
        order.push("piper");
    }
    if !order.contains(&"rhvoice") {
        order.push("rhvoice");
    }
    if !order.contains(&"flite") {
        order.push("flite");
    }
    if !order.contains(&"rutts") {
        order.push("rutts");
    }
    if !order.contains(&"tgspeechbox") {
        order.push("tgspeechbox");
    }
    order
}

#[cfg(target_os = "macos")]
fn native_registry_engine_id() -> Option<&'static str> {
    Some("macos")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_registry_engine_id() -> Option<&'static str> {
    None
}

fn piper_is_configured(model: Option<&str>) -> bool {
    model.is_some_and(|model| !model.is_empty())
        || std::env::var_os("OMNIVOX_PIPER_MODEL").is_some_and(|model| !model.is_empty())
}

#[cfg(feature = "piper")]
fn configured_piper_helper(requested: &str, model: Option<&str>) -> Option<HelperEngineConfig> {
    if requested != "piper" && !piper_is_configured(model) {
        return None;
    }
    match piper_helper_config(model) {
        Ok(config) => Some(config),
        Err(error) => {
            warn!("Piper TTS helper not available: {error}");
            None
        }
    }
}

#[cfg(not(feature = "piper"))]
fn configured_piper_helper(requested: &str, model: Option<&str>) -> Option<HelperEngineConfig> {
    if requested == "piper" || piper_is_configured(model) {
        warn!(
            "Piper was requested or configured but omnivox was built without Piper support. \
             Rebuild with --features piper."
        );
    }
    None
}

fn configured_helper_configs(requested: &str, model: Option<&str>) -> Vec<HelperEngineConfig> {
    let mut configs = Vec::with_capacity(5);
    if let Some(piper) = configured_piper_helper(requested, model) {
        configs.push(piper);
    }
    configs.extend(companion_helper_configs());
    configs
}

/// Create one exact TTS engine for a diagnostic action.
///
/// `engine_name` may be empty (use `OMNIVOX_ENGINE` env var or platform default),
/// `"native"`, `"espeak"`, `"piper"`, `"rhvoice"`, `"flite"`, `"rutts"`, or
/// `"tgspeechbox"`.
/// Windows also accepts `"winrt"`, `"eloquence"`, and `"dectalk"`; macOS
/// accepts `"macos"`. An explicitly requested unavailable engine is an error,
/// so diagnostic results cannot silently describe a fallback engine.
/// `piper_model` is the path to a `.onnx` model file; if `None`,
/// `OMNIVOX_PIPER_MODEL` is consulted.
pub fn create_engine(engine_name: &str, _piper_model: Option<&str>) -> Result<Arc<dyn TtsEngine>> {
    let forced = requested_engine(engine_name);

    if forced == "piper" {
        #[cfg(feature = "piper")]
        {
            match piper_helper_config(_piper_model)
                .and_then(|config| HelperTtsEngine::new(config).map_err(anyhow::Error::from))
            {
                Ok(engine) => {
                    info!("Using Piper neural TTS helper");
                    return Ok(Arc::new(engine));
                }
                Err(error) => anyhow::bail!("Piper TTS helper is not available: {error}"),
            }
        }
        #[cfg(not(feature = "piper"))]
        anyhow::bail!(
            "Piper was requested but omnivox was built without Piper support; \
             rebuild with --features piper"
        );
    }

    if matches!(
        forced.as_str(),
        "rhvoice" | "flite" | "rutts" | "tgspeechbox"
    ) {
        let (engine_id, environment_variable) = match forced.as_str() {
            "rhvoice" => ("rhvoice", "OMNIVOX_RHVOICE_HELPER"),
            "flite" => ("flite", "OMNIVOX_FLITE_HELPER"),
            "rutts" => ("rutts", "OMNIVOX_RUTTS_HELPER"),
            "tgspeechbox" => ("tgspeechbox", "OMNIVOX_TGSPEECHBOX_HELPER"),
            _ => unreachable!(),
        };
        match companion_helper_config(engine_id, environment_variable)
            .ok_or_else(|| anyhow::anyhow!("the {engine_id} helper was not found"))
            .and_then(|config| HelperTtsEngine::new(config).map_err(anyhow::Error::from))
        {
            Ok(engine) => {
                info!("Using {engine_id} TTS helper");
                return Ok(Arc::new(engine));
            }
            Err(error) => anyhow::bail!("{engine_id} TTS helper is not available: {error}"),
        }
    }

    if matches!(forced.as_str(), "eloquence" | "dectalk") {
        #[cfg(target_os = "windows")]
        {
            let (environment_variable, adjacent_filename) = match forced.as_str() {
                "eloquence" => ("OMNIVOX_ELOQUENCE_HELPER", "OmnivoxEloquenceHelper32.exe"),
                "dectalk" => ("OMNIVOX_DECTALK_HELPER", "OmnivoxDectalkHelper32.exe"),
                _ => unreachable!(),
            };
            let config = helper_config(&forced, environment_variable, adjacent_filename)
                .ok_or_else(|| anyhow::anyhow!("the {forced} helper was not found"))?;
            let engine = HelperTtsEngine::new(config).map_err(|error| {
                anyhow::anyhow!("{forced} TTS helper is not available: {error}")
            })?;
            info!("Using {forced} TTS helper");
            return Ok(Arc::new(engine));
        }
        #[cfg(not(target_os = "windows"))]
        anyhow::bail!("{forced} is available only on Windows");
    }

    #[cfg(target_os = "macos")]
    if forced.is_empty() {
        match MacOsTtsEngine::new() {
            Ok(engine) => {
                info!("Using macOS AVSpeechSynthesizer engine");
                return Ok(Arc::new(engine));
            }
            Err(error) => warn!("macOS TTS is not available: {error}; trying eSpeak NG"),
        }
    }

    #[cfg(target_os = "macos")]
    if matches!(forced.as_str(), "native" | "macos") {
        return match MacOsTtsEngine::new() {
            Ok(engine) => {
                info!("Using macOS AVSpeechSynthesizer engine");
                Ok(Arc::new(engine))
            }
            Err(error) => anyhow::bail!("macOS TTS is not available: {error}"),
        };
    }

    #[cfg(target_os = "windows")]
    if forced.is_empty() {
        match WindowsTtsEngine::new() {
            Ok(engine) => {
                info!("Using Windows WinRT engine");
                return Ok(Arc::new(engine));
            }
            Err(error) => warn!("Windows WinRT is not available: {error}; trying eSpeak NG"),
        }
    }

    #[cfg(target_os = "windows")]
    if matches!(forced.as_str(), "native" | "winrt") {
        return match WindowsTtsEngine::new() {
            Ok(engine) => {
                info!("Using Windows WinRT engine");
                Ok(Arc::new(engine))
            }
            Err(error) => anyhow::bail!("Windows WinRT is not available: {error}"),
        };
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let use_espeak = matches!(forced.as_str(), "" | "native" | "espeak");
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let use_espeak = matches!(forced.as_str(), "" | "espeak");

    if !use_espeak {
        anyhow::bail!("unknown TTS engine: {forced}");
    }

    match EspeakTtsEngine::new() {
        Ok(engine) => {
            info!("Using espeak-ng engine");
            Ok(Arc::new(engine))
        }
        Err(error) => anyhow::bail!("eSpeak NG is not available: {error}"),
    }
}

#[cfg(not(target_os = "windows"))]
fn isolate_server_engine(
    engine: Arc<dyn TtsEngine>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Arc<dyn TtsEngine> {
    if !matches!(
        engine.descriptor().id.as_str(),
        "piper" | "rhvoice" | "flite" | "rutts" | "tgspeechbox"
    ) {
        return engine;
    }
    Arc::new(IsolatedTtsEngine::new(engine, generation, isolation_budget))
}

#[cfg(feature = "piper")]
fn piper_helper_config(model: Option<&str>) -> Result<HelperEngineConfig> {
    let model = model
        .map(str::to_owned)
        .or_else(|| std::env::var("OMNIVOX_PIPER_MODEL").ok())
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no model path was provided; set OMNIVOX_PIPER_MODEL or use --piper-model"
            )
        })?;
    let helper_filename = format!("omnivox-piper-helper{}", std::env::consts::EXE_SUFFIX);
    let candidates = [
        PathBuf::from("piper").join(&helper_filename),
        PathBuf::from(&helper_filename),
    ];
    let mut config = helper_config_with_candidates("piper", "OMNIVOX_PIPER_HELPER", &candidates)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} was not found in the Piper companion directory or beside Omnivox; set \
                 OMNIVOX_PIPER_HELPER",
                helper_filename
            )
        })?;
    config.arguments.push("--model".into());
    config.arguments.push(model.into());
    config.startup_timeout = Duration::from_secs(60);
    config.synthesis_idle_timeout = Duration::from_secs(60);
    Ok(config)
}

/// Human-readable name of the platform-native TTS backend.
pub fn native_engine_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos (AVSpeechSynthesizer)"
    }
    #[cfg(target_os = "windows")]
    {
        "winrt (Windows SpeechSynthesizer)"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "none (espeak-ng is the only backend)"
    }
}

/// Apply `OMNIVOX_AUDIO_TARGET` to every output stream in this process.
pub fn apply_audio_target_env(state: &mut TtsState) {
    if let Ok(target) = std::env::var("OMNIVOX_AUDIO_TARGET") {
        if let Some(channel_mode) = ChannelMode::parse(&target) {
            info!("Setting audio target from env: {}", target);
            state.set_process_channel_mode(channel_mode);
        } else {
            warn!("Invalid OMNIVOX_AUDIO_TARGET value: {}", target);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::create_engines;
    use super::{
        companion_helper_candidates, engine_preference_order, helper_synthesis_idle_timeout,
        resolve_adjacent_helper, start_helper_initializations_with,
        tgspeechbox_descriptor_cache_file_name, HelperEngineConfig,
    };
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::time::Duration;

    #[test]
    fn failed_helper_startup_retains_an_unavailable_inventory_entry() {
        let pending = start_helper_initializations_with(
            vec![HelperEngineConfig::new("dectalk", "unused-helper")],
            |_| -> super::HelperInitializationResult {
                Err(omnivox_tts::helper_engine::HelperEngineError::Transport(
                    "test runtime is missing".to_owned(),
                ))
            },
        );
        let mut registry = omnivox_tts::engine_registry::EngineRegistry::new();
        super::register_initialized_helpers(
            &mut registry,
            pending,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            Arc::new(crate::engine_execution::IsolationBudget::new()),
        )
        .unwrap();
        let inventory = registry.inventory();
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].id, "dectalk");
        assert!(inventory[0].voices.is_empty());
        assert!(matches!(&inventory[0].availability,
            omnivox_tts::contracts::Availability::Unavailable { reason }
                if reason.contains("test runtime is missing")));
        assert!(registry.engine("dectalk").is_none());
    }

    #[test]
    fn helper_initialization_starts_concurrently_and_retains_order() {
        let configs = ["first", "second"]
            .into_iter()
            .map(|engine_id| HelperEngineConfig::new(engine_id, "unused-helper"))
            .collect();
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_release = Arc::clone(&release);
        let pending = start_helper_initializations_with(configs, move |config| {
            started_tx.send(config.engine_id.clone()).unwrap();
            let (lock, changed) = &*worker_release;
            let released = lock.lock().unwrap();
            let _ = changed
                .wait_timeout_while(released, Duration::from_secs(2), |released| !*released)
                .unwrap();
            config.engine_id
        });

        assert_eq!(pending.len(), 2);
        assert_eq!(
            pending
                .iter()
                .map(|initialization| initialization.engine_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let mut started = vec![
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        ];
        started.sort();
        assert_eq!(started, ["first", "second"]);

        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        let completed = pending
            .into_iter()
            .map(|initialization| initialization.handle.unwrap().join().unwrap().0)
            .collect::<Vec<_>>();
        assert_eq!(completed, ["first", "second"]);
    }

    #[test]
    fn eloquence_helper_fails_over_after_a_short_idle_timeout() {
        assert_eq!(
            helper_synthesis_idle_timeout("eloquence"),
            Duration::from_millis(500)
        );
        assert_eq!(
            helper_synthesis_idle_timeout("dectalk"),
            Duration::from_secs(60)
        );
        assert_eq!(
            helper_synthesis_idle_timeout("piper"),
            Duration::from_secs(60)
        );
        assert_eq!(
            helper_synthesis_idle_timeout("rhvoice"),
            Duration::from_secs(60)
        );
        assert_eq!(
            helper_synthesis_idle_timeout("flite"),
            Duration::from_secs(60)
        );
        assert_eq!(
            helper_synthesis_idle_timeout("rutts"),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn tgspeechbox_inventory_follows_the_native_sample_rate() {
        assert_eq!(
            tgspeechbox_descriptor_cache_file_name(None),
            "VOICE-INVENTORY-44100.json"
        );
        assert_eq!(
            tgspeechbox_descriptor_cache_file_name(Some(std::ffi::OsStr::new("44100"))),
            "VOICE-INVENTORY-44100.json"
        );
        assert_eq!(
            tgspeechbox_descriptor_cache_file_name(Some(std::ffi::OsStr::new("22050"))),
            "VOICE-INVENTORY-22050.json"
        );
    }

    #[test]
    fn piper_companion_directory_precedes_legacy_adjacent_helper() {
        let root = std::env::temp_dir().join(format!(
            "omnivox-piper-helper-resolution-test-{}",
            std::process::id()
        ));
        let companion = root.join("piper/omnivox-piper-helper");
        let legacy = root.join("omnivox-piper-helper");
        std::fs::create_dir_all(companion.parent().unwrap()).unwrap();
        std::fs::write(&companion, b"companion").unwrap();
        std::fs::write(&legacy, b"legacy").unwrap();

        let candidates = [
            PathBuf::from("piper/omnivox-piper-helper"),
            PathBuf::from("omnivox-piper-helper"),
        ];
        assert_eq!(
            resolve_adjacent_helper(&root.join("omnivox"), &candidates),
            Some(companion.clone())
        );

        std::fs::remove_file(companion).unwrap();
        assert_eq!(
            resolve_adjacent_helper(&root.join("omnivox"), &candidates),
            Some(legacy)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_defaults_to_winrt_with_espeak_fallback() {
        assert_eq!(
            engine_preference_order("", Some("winrt")),
            [
                "winrt",
                "espeak",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("native", Some("winrt")),
            [
                "winrt",
                "espeak",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
    }

    #[test]
    fn windows_honours_explicit_engine_preferences() {
        assert_eq!(
            engine_preference_order("espeak", Some("winrt")),
            [
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("piper", Some("winrt")),
            &[
                "piper",
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("rhvoice", Some("winrt")),
            &[
                "rhvoice",
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "piper",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("flite", Some("winrt")),
            &[
                "flite",
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("rutts", Some("winrt")),
            &[
                "rutts",
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("eloquence", Some("winrt")),
            &[
                "eloquence",
                "espeak",
                "winrt",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("dectalk", Some("winrt")),
            &[
                "dectalk",
                "espeak",
                "winrt",
                "eloquence",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("tgspeechbox", Some("winrt")),
            &[
                "tgspeechbox",
                "espeak",
                "winrt",
                "eloquence",
                "dectalk",
                "piper",
                "rhvoice",
                "flite",
                "rutts"
            ]
        );
    }

    #[test]
    fn macos_retains_native_and_espeak_for_each_preference() {
        assert_eq!(
            engine_preference_order("", Some("macos")),
            [
                "macos",
                "espeak",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("espeak", Some("macos")),
            [
                "espeak",
                "macos",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("piper", Some("macos")),
            [
                "piper",
                "espeak",
                "macos",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_server_registers_native_when_espeak_is_preferred() {
        let created = create_engines("espeak", None, Arc::new(AtomicU64::new(0)))
            .expect("macOS and eSpeak engines should initialize");
        assert_eq!(created.preferred.descriptor().id, "espeak");
        assert!(created.registry.engine("macos").is_some());
        assert!(created.registry.engine("espeak").is_some());
    }

    #[test]
    fn linux_retains_espeak_when_piper_is_preferred() {
        assert_eq!(
            engine_preference_order("", None),
            [
                "espeak",
                "piper",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
        assert_eq!(
            engine_preference_order("piper", None),
            [
                "piper",
                "espeak",
                "rhvoice",
                "flite",
                "rutts",
                "tgspeechbox"
            ]
        );
    }

    #[test]
    fn companion_directory_precedes_legacy_adjacent_helper() {
        let root = std::env::temp_dir().join(format!(
            "omnivox-rhvoice-helper-resolution-test-{}",
            std::process::id()
        ));
        let companion = root.join(format!(
            "rhvoice/omnivox-rhvoice-helper{}",
            std::env::consts::EXE_SUFFIX
        ));
        let legacy = root.join(format!(
            "omnivox-rhvoice-helper{}",
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::create_dir_all(companion.parent().unwrap()).unwrap();
        std::fs::write(&companion, b"companion").unwrap();
        std::fs::write(&legacy, b"legacy").unwrap();

        let candidates = companion_helper_candidates("rhvoice");
        assert_eq!(
            resolve_adjacent_helper(&root.join("omnivox"), &candidates),
            Some(companion)
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
