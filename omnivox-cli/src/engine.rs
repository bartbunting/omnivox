//! TTS engine selection and creation.

use anyhow::Result;
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::engine_configuration::{LaunchEnvironment, Platform};
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
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};
use crate::voice_library::StartupLibrary;

mod startup;
pub(crate) use startup::EngineStartup;

const TGSPEECHBOX_SAMPLE_RATE_ENVIRONMENT_VARIABLE: &str = "OMNIVOX_TGSPEECHBOX_SAMPLE_RATE";
const TGSPEECHBOX_22050_CACHE_FILE_NAME: &str = "VOICE-INVENTORY-22050.json";
const TGSPEECHBOX_44100_CACHE_FILE_NAME: &str = "VOICE-INVENTORY-44100.json";

/// Engines initialized for one server session.
pub struct CreatedEngines {
    pub preferred: Arc<dyn TtsEngine>,
    pub registry: EngineRegistry,
    pub speech: omnivox_tts::engine_configuration::SpeechConfiguration,
    pub audio: omnivox_core::settings::AudioOutputSettings,
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
    voice_library: Option<&str>,
    config_dir: Option<&str>,
    generation: Arc<AtomicU64>,
) -> Result<CreatedEngines> {
    let startup = EngineStartup::read(engine_name, piper_model, voice_library, config_dir)?;
    let library = startup.library.as_ref();
    let isolation_budget = Arc::new(IsolationBudget::new());
    let mut registry = startup.registry()?;
    let pending = start_helper_initializations(startup.helper_configs(false), library);
    for id in [native_registry_engine_id(), Some("espeak")]
        .into_iter()
        .flatten()
    {
        if !startup.permits_construction(id) {
            continue;
        }
        match construct_in_process(id) {
            Ok(engine) => {
                #[cfg(windows)]
                let engine = if id == "winrt" {
                    Arc::new(IsolatedTtsEngine::new(
                        engine,
                        generation.clone(),
                        isolation_budget.clone(),
                    )) as Arc<dyn TtsEngine>
                } else {
                    engine
                };
                registry.register(engine)?;
                info!(engine_id = id, "Registered native engine");
            }
            Err(error) => warn!(engine_id = id, %error, "Native engine unavailable"),
        }
    }
    let external = omnivox_tts::helper_engine::initialize_external_helpers(
        startup.helper_configs(true),
        startup.external_priority(),
    );
    register_initialized_helpers(
        &mut registry,
        pending,
        generation.clone(),
        isolation_budget.clone(),
        library,
    )?;
    for initialized in external {
        match (initialized.engine, initialized.result) {
            (Some(engine), Ok(_)) => {
                registry.register(Arc::new(IsolatedTtsEngine::new(
                    engine,
                    generation.clone(),
                    isolation_budget.clone(),
                )))?;
            }
            (engine, Err(reason)) => {
                let generation = generation.clone();
                let isolation_budget = isolation_budget.clone();
                let retry_reason = reason.clone();
                registry.register_unavailable(
                    EngineDescriptor::unavailable(initialized.engine_id, reason),
                    move || {
                        let engine = engine.as_ref().ok_or_else(|| retry_reason.clone())?;
                        engine
                            .initialize_before(
                                Instant::now()
                                    + omnivox_tts::helper_engine::EXTERNAL_STARTUP_BUDGET,
                            )
                            .map_err(|e| e.to_string())?;
                        Ok(Arc::new(IsolatedTtsEngine::new(
                            engine.clone(),
                            generation.clone(),
                            isolation_budget.clone(),
                        )) as Arc<dyn TtsEngine>)
                    },
                )?;
            }
            (None, Ok(_)) => unreachable!("successful initialization owns its engine"),
        }
    }
    let preferred = startup
        .order()
        .iter()
        .filter_map(|id| registry.engine(id))
        .find(|engine| engine.descriptor().can_synthesize())
        .ok_or_else(|| anyhow::anyhow!("No eligible TTS engine available"))?;
    if library.is_some() {
        crate::voice_library::preflight_responses(&registry, &preferred.descriptor().id)?;
    }
    info!(
        engine_id = preferred.descriptor().id,
        "Selected startup engine"
    );
    Ok(CreatedEngines {
        preferred,
        registry,
        speech: startup.snapshot.resolved().speech.clone(),
        audio: startup.snapshot.resolved().audio,
    })
}

fn construct_in_process(id: &str) -> Result<Arc<dyn TtsEngine>> {
    match id {
        "espeak" => Ok(Arc::new(EspeakTtsEngine::new()?)),
        #[cfg(windows)]
        "winrt" => Ok(Arc::new(WindowsTtsEngine::new()?)),
        #[cfg(target_os = "macos")]
        "macos" => Ok(Arc::new(MacOsTtsEngine::new()?)),
        _ => anyhow::bail!("engine is unavailable on this platform"),
    }
}

struct PendingHelperInitialization<T> {
    engine_id: String,
    helper_path: PathBuf,
    config: HelperEngineConfig,
    owner: Arc<Mutex<Option<Arc<HelperTtsEngine>>>>,
    handle: std::io::Result<JoinHandle<(T, Duration)>>,
}

type HelperInitializationResult =
    Result<Arc<HelperTtsEngine>, omnivox_tts::helper_engine::HelperEngineError>;
type PendingHelper = PendingHelperInitialization<HelperInitializationResult>;

fn start_helper_initializations(
    configs: Vec<HelperEngineConfig>,
    library: Option<&StartupLibrary>,
) -> Vec<PendingHelper> {
    let library = library.cloned();
    start_helper_initializations_with(configs, move |config, owner| {
        let engine_id = config.engine_id.clone();
        let helper_path = config.program.clone();
        if let Some(library) = &library {
            library.verify_assets(&engine_id).map_err(|error| {
                omnivox_tts::helper_engine::HelperEngineError::Transport(error.to_string())
            })?;
        }
        let result = initialize_server_helper(config, owner);
        if engine_id == "tgspeechbox" {
            if let Ok(engine) = &result {
                spawn_tgspeechbox_prewarm(Arc::clone(engine), helper_path);
            }
        }
        result
    })
}

fn initialize_server_helper(
    config: HelperEngineConfig,
    owner: &Mutex<Option<Arc<HelperTtsEngine>>>,
) -> HelperInitializationResult {
    let (engine, deferred) = prepare_server_helper(config)?;
    let engine = Arc::new(engine);
    *owner.lock().unwrap() = Some(engine.clone());
    if !deferred {
        engine.prewarm_connection()?;
    }
    Ok(engine)
}

fn prepare_server_helper(
    config: HelperEngineConfig,
) -> Result<(HelperTtsEngine, bool), omnivox_tts::helper_engine::HelperEngineError> {
    if config.engine_id != "tgspeechbox" {
        return HelperTtsEngine::prepare(config).map(|engine| (engine, false));
    }

    let helper_directory = config.program.parent().unwrap_or_else(|| Path::new(""));
    let cache_file_name = tgspeechbox_descriptor_cache_file_name(
        config
            .environment
            .get(TGSPEECHBOX_SAMPLE_RATE_ENVIRONMENT_VARIABLE),
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
            HelperTtsEngine::new_deferred(config, descriptor).map(|engine| (engine, true))
        }
        Err(error) => {
            warn!(
                engine_id = config.engine_id,
                helper = %config.program.display(),
                cache = %cache_path.display(),
                %error,
                "Cached voice inventory is unavailable; initializing helper eagerly"
            );
            HelperTtsEngine::prepare(config).map(|engine| (engine, false))
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
    F: Fn(HelperEngineConfig, &Mutex<Option<Arc<HelperTtsEngine>>>) -> T + Send + Sync + 'static,
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
            let owner = Arc::new(Mutex::new(None));
            let thread_owner = owner.clone();
            let handle = thread::Builder::new().name(thread_name).spawn(move || {
                let started_at = Instant::now();
                let result = initialize(thread_config, &thread_owner);
                (result, started_at.elapsed())
            });
            PendingHelperInitialization {
                engine_id,
                helper_path,
                config,
                owner,
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
    library: Option<&StartupLibrary>,
) -> Result<()> {
    for initialization in pending {
        let PendingHelperInitialization {
            engine_id,
            helper_path,
            config,
            owner,
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
            })
            .and_then(|(engine, elapsed)| {
                if let Some(library) = library {
                    library
                        .validate_descriptor(&engine.descriptor())
                        .map_err(|e| e.to_string())?;
                }
                Ok((engine, elapsed))
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
                let library = library.cloned();
                registry.register_unavailable(
                    EngineDescriptor::unavailable(&engine_id, reason),
                    move || {
                        // A rescan must perform live discovery, including for
                        // helpers that normally permit a deferred cached inventory.
                        if let Some(library) = &library {
                            library
                                .verify_assets(&config.engine_id)
                                .map_err(|error| error.to_string())?;
                        }
                        let engine = {
                            let mut retained =
                                owner.lock().map_err(|_| "helper owner lock poisoned")?;
                            if retained.is_none() {
                                *retained = Some(Arc::new(
                                    HelperTtsEngine::prepare(config.clone())
                                        .map_err(|e| e.to_string())?,
                                ));
                            }
                            retained.as_ref().unwrap().clone()
                        };
                        engine
                            .refresh_connection()
                            .map_err(|error| error.to_string())?;
                        if let Some(library) = &library {
                            library
                                .validate_descriptor(&engine.descriptor())
                                .map_err(|error| error.to_string())?;
                        }
                        Ok(Arc::new(IsolatedTtsEngine::new(
                            engine,
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
        "mbrola" => order.push("mbrola"),
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

#[cfg(target_os = "windows")]
fn native_registry_engine_id() -> Option<&'static str> {
    Some("winrt")
}

#[cfg(target_os = "macos")]
fn native_registry_engine_id() -> Option<&'static str> {
    Some("macos")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn native_registry_engine_id() -> Option<&'static str> {
    None
}

#[cfg(test)]
mod library_tests;

#[cfg(test)]
mod configuration_tests;

/// Create one exact TTS engine for a diagnostic action.
///
/// Names resolve through the same shipped/external registrations as server
/// startup. Empty selection uses the captured environment and startup policy;
/// `native` remains a compatibility alias. An explicitly requested unavailable
/// engine is an error, so diagnostics cannot describe a fallback engine.
/// `piper_model` is the path to a `.onnx` model file; if `None`,
/// `OMNIVOX_PIPER_MODEL` is consulted.
pub fn create_engine(
    engine_name: &str,
    piper_model: Option<&str>,
    voice_library: Option<&str>,
    config_dir: Option<&str>,
) -> Result<Arc<dyn TtsEngine>> {
    create_diagnostic_engine(engine_name, piper_model, voice_library, config_dir)
        .map(|(engine, _, _)| engine)
}

/// Return speech and audio defaults from the same capture that selected the engine.
pub fn create_diagnostic_engine(
    engine_name: &str,
    piper_model: Option<&str>,
    voice_library: Option<&str>,
    config_dir: Option<&str>,
) -> Result<(
    Arc<dyn TtsEngine>,
    omnivox_tts::engine_configuration::SpeechConfiguration,
    omnivox_core::settings::AudioOutputSettings,
)> {
    let startup = EngineStartup::read(engine_name, piper_model, voice_library, config_dir)?;
    let ids = if startup.snapshot.requested().is_empty() {
        startup.order()
    } else {
        vec![if startup.snapshot.requested() == "native" {
            native_registry_engine_id().unwrap_or("espeak").to_owned()
        } else {
            startup.snapshot.requested().to_owned()
        }]
    };
    let mut last_error = None;
    for id in ids {
        let attempt = (|| -> Result<Arc<dyn TtsEngine>> {
            let registration = startup
                .snapshot
                .resolved()
                .registration(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown TTS engine: {id}"))?;
            anyhow::ensure!(
                registration.unavailable.is_none(),
                "{id}: {}",
                registration.unavailable.as_deref().unwrap_or_default()
            );
            if let Some(library) = &startup.library {
                library.verify_assets(&id)?;
            }
            let engine = if let Some(config) = &registration.helper {
                let engine = Arc::new(HelperTtsEngine::prepare(config.clone())?);
                if registration.origin
                    == omnivox_tts::engine_configuration::EngineOrigin::ExternalHelper
                {
                    engine.initialize_before(
                        Instant::now() + omnivox_tts::helper_engine::EXTERNAL_STARTUP_BUDGET,
                    )?;
                } else {
                    engine.prewarm_connection()?;
                }
                engine as Arc<dyn TtsEngine>
            } else {
                construct_in_process(&id)?
            };
            if let Some(library) = &startup.library {
                library.validate_descriptor(&engine.descriptor())?;
                Ok(library.eligibility.guard_engine(engine))
            } else {
                Ok(engine)
            }
        })();
        match attempt {
            Ok(engine) => {
                return Ok((
                    engine,
                    startup.snapshot.resolved().speech.clone(),
                    startup.snapshot.resolved().audio,
                ))
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No eligible TTS engine available")))
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
        "none (default: espeak-ng; optional engines use helpers)"
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
        engine_preference_order, start_helper_initializations_with,
        tgspeechbox_descriptor_cache_file_name, HelperEngineConfig,
    };
    use omnivox_tts::engine_configuration::{shipped, Platform};
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::AtomicU64;
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::time::Duration;

    #[test]
    fn failed_helper_startup_retains_an_unavailable_inventory_entry() {
        let pending = start_helper_initializations_with(
            vec![HelperEngineConfig::new("dectalk", "unused-helper")],
            |_, _| -> super::HelperInitializationResult {
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
            None,
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
        let pending = start_helper_initializations_with(configs, move |config, _| {
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
            shipped::definition("eloquence")
                .unwrap()
                .synthesis_idle_timeout(),
            Duration::from_millis(500)
        );
        assert_eq!(
            shipped::definition("dectalk")
                .unwrap()
                .synthesis_idle_timeout(),
            Duration::from_secs(60)
        );
        assert_eq!(
            shipped::definition("piper")
                .unwrap()
                .synthesis_idle_timeout(),
            Duration::from_secs(60)
        );
        assert_eq!(
            shipped::definition("rhvoice")
                .unwrap()
                .synthesis_idle_timeout(),
            Duration::from_secs(60)
        );
        assert_eq!(
            shipped::definition("flite")
                .unwrap()
                .synthesis_idle_timeout(),
            Duration::from_secs(60)
        );
        assert_eq!(
            shipped::definition("rutts")
                .unwrap()
                .synthesis_idle_timeout(),
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
            shipped::resolve_adjacent(&root.join("omnivox"), &candidates),
            Some(companion.clone())
        );

        std::fs::remove_file(companion).unwrap();
        assert_eq!(
            shipped::resolve_adjacent(&root.join("omnivox"), &candidates),
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
        let created = create_engines("espeak", None, None, None, Arc::new(AtomicU64::new(0)))
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

        let candidates = shipped::definition("rhvoice")
            .unwrap()
            .helper_candidates(Platform::native());
        assert_eq!(
            shipped::resolve_adjacent(&root.join("omnivox"), &candidates),
            Some(companion)
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
