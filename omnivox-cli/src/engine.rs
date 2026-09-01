//! TTS engine selection and creation.

use anyhow::Result;
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::espeak::EspeakTtsEngine;
use omnivox_tts::helper_engine::{HelperEngineConfig, HelperTtsEngine};
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::TtsEngine;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};

const ELOQUENCE_SYNTHESIS_IDLE_TIMEOUT: Duration = Duration::from_millis(500);
const NATIVE_HELPER_SYNTHESIS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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
/// Server mode eagerly initializes the available built-in engines so runtime
/// routing can retain fallbacks. Piper remains opt-in through model
/// configuration because starting its helper loads a comparatively large
/// voice model.
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

    register_configured_piper(
        &mut registry,
        &requested,
        piper_model,
        Arc::clone(&generation),
        Arc::clone(&isolation_budget),
    )?;
    register_companion_helpers(&mut registry, generation, isolation_budget)?;

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

    register_optional_helper(
        &mut registry,
        helper_config(
            "eloquence",
            "OMNIVOX_ELOQUENCE_HELPER",
            "OmnivoxEloquenceHelper32.exe",
        ),
        Arc::clone(&generation),
        Arc::clone(&isolation_budget),
    )?;
    register_optional_helper(
        &mut registry,
        helper_config(
            "dectalk",
            "OMNIVOX_DECTALK_HELPER",
            "OmnivoxDectalkHelper32.exe",
        ),
        Arc::clone(&generation),
        Arc::clone(&isolation_budget),
    )?;

    register_configured_piper(
        &mut registry,
        &forced,
        piper_model,
        Arc::clone(&generation),
        Arc::clone(&isolation_budget),
    )?;
    register_companion_helpers(&mut registry, generation, isolation_budget)?;

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

fn register_optional_helper(
    registry: &mut EngineRegistry,
    config: Option<HelperEngineConfig>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    let engine_id = config.engine_id.clone();
    let helper_path = config.program.clone();
    match HelperTtsEngine::new(config) {
        Ok(engine) => {
            let engine: Arc<dyn TtsEngine> = Arc::new(engine);
            registry.register(Arc::new(IsolatedTtsEngine::new(
                engine,
                generation,
                isolation_budget,
            )))?;
            info!(
                "Registered {} helper engine: {}",
                engine_id,
                helper_path.display()
            );
        }
        Err(error) => warn!(
            "{} helper at {} is not available: {}",
            engine_id,
            helper_path.display(),
            error
        ),
    }
    Ok(())
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

fn register_companion_helpers(
    registry: &mut EngineRegistry,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<()> {
    for (engine_id, environment_variable) in [
        ("rhvoice", "OMNIVOX_RHVOICE_HELPER"),
        ("flite", "OMNIVOX_FLITE_HELPER"),
        ("rutts", "OMNIVOX_RUTTS_HELPER"),
    ] {
        register_optional_helper(
            registry,
            companion_helper_config(engine_id, environment_variable),
            Arc::clone(&generation),
            Arc::clone(&isolation_budget),
        )?;
    }
    Ok(())
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
    let mut order = Vec::with_capacity(6);
    match requested {
        "espeak" => order.push("espeak"),
        "piper" => order.push("piper"),
        "rhvoice" => order.push("rhvoice"),
        "flite" => order.push("flite"),
        "rutts" => order.push("rutts"),
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
fn register_configured_piper(
    registry: &mut EngineRegistry,
    requested: &str,
    model: Option<&str>,
    generation: Arc<AtomicU64>,
    isolation_budget: Arc<IsolationBudget>,
) -> Result<()> {
    if requested != "piper" && !piper_is_configured(model) {
        return Ok(());
    }
    match piper_helper_config(model) {
        Ok(config) => {
            register_optional_helper(registry, Some(config), generation, isolation_budget)
        }
        Err(error) => {
            warn!("Piper TTS helper not available: {error}");
            Ok(())
        }
    }
}

#[cfg(not(feature = "piper"))]
fn register_configured_piper(
    _registry: &mut EngineRegistry,
    requested: &str,
    model: Option<&str>,
    _generation: Arc<AtomicU64>,
    _isolation_budget: Arc<IsolationBudget>,
) -> Result<()> {
    if requested == "piper" || piper_is_configured(model) {
        warn!(
            "Piper was requested or configured but omnivox was built without Piper support. \
             Rebuild with --features piper."
        );
    }
    Ok(())
}

/// Create a TTS engine by name, falling back through the platform default to espeak-ng.
///
/// `engine_name` may be empty (use `OMNIVOX_ENGINE` env var or platform default),
/// `"espeak"`, `"piper"`, `"rhvoice"`, `"flite"`, or `"rutts"`.
/// `piper_model` is the path to a `.onnx` model file; if `None`,
/// `OMNIVOX_PIPER_MODEL` is consulted.
pub fn create_engine(engine_name: &str, _piper_model: Option<&str>) -> Result<Arc<dyn TtsEngine>> {
    let forced = if engine_name.is_empty() {
        std::env::var("OMNIVOX_ENGINE").unwrap_or_default()
    } else {
        engine_name.to_string()
    };

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
                Err(error) => {
                    warn!("Piper TTS helper not available: {error}; falling back to espeak-ng")
                }
            }
        }
        #[cfg(not(feature = "piper"))]
        warn!(
            "OMNIVOX_ENGINE=piper but omnivox was built without piper support. \
             Rebuild with --features piper. Falling back to espeak-ng."
        );
    }

    if matches!(forced.as_str(), "rhvoice" | "flite" | "rutts") {
        let (engine_id, environment_variable) = match forced.as_str() {
            "rhvoice" => ("rhvoice", "OMNIVOX_RHVOICE_HELPER"),
            "flite" => ("flite", "OMNIVOX_FLITE_HELPER"),
            "rutts" => ("rutts", "OMNIVOX_RUTTS_HELPER"),
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
            Err(error) => {
                warn!("{engine_id} TTS helper not available: {error}; falling back to espeak-ng")
            }
        }
    }

    if !matches!(
        forced.as_str(),
        "espeak" | "piper" | "rhvoice" | "flite" | "rutts"
    ) {
        #[cfg(target_os = "macos")]
        match MacOsTtsEngine::new() {
            Ok(engine) => {
                info!("Using macOS AVSpeechSynthesizer engine");
                return Ok(Arc::new(engine));
            }
            Err(e) => warn!("macOS TTS not available: {}, falling back to espeak-ng", e),
        }

        #[cfg(target_os = "windows")]
        match WindowsTtsEngine::new() {
            Ok(engine) => {
                info!("Using Windows WinRT engine");
                return Ok(Arc::new(engine));
            }
            Err(e) => warn!(
                "Windows WinRT not available: {}, falling back to espeak-ng",
                e
            ),
        }
    }

    match EspeakTtsEngine::new() {
        Ok(engine) => {
            info!("Using espeak-ng engine");
            Ok(Arc::new(engine))
        }
        Err(e) => anyhow::bail!("No TTS engine available: {}", e),
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
        "piper" | "rhvoice" | "flite" | "rutts"
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
        resolve_adjacent_helper,
    };
    use std::path::PathBuf;
    #[cfg(target_os = "macos")]
    use std::sync::atomic::AtomicU64;
    #[cfg(target_os = "macos")]
    use std::sync::Arc;
    use std::time::Duration;

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
            ["winrt", "espeak", "piper", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("native", Some("winrt")),
            ["winrt", "espeak", "piper", "rhvoice", "flite", "rutts"]
        );
    }

    #[test]
    fn windows_honours_explicit_engine_preferences() {
        assert_eq!(
            engine_preference_order("espeak", Some("winrt")),
            ["espeak", "winrt", "piper", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("piper", Some("winrt")),
            &["piper", "espeak", "winrt", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("rhvoice", Some("winrt")),
            &["rhvoice", "espeak", "winrt", "piper", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("flite", Some("winrt")),
            &["flite", "espeak", "winrt", "piper", "rhvoice", "rutts"]
        );
        assert_eq!(
            engine_preference_order("rutts", Some("winrt")),
            &["rutts", "espeak", "winrt", "piper", "rhvoice", "flite"]
        );
    }

    #[test]
    fn macos_retains_native_and_espeak_for_each_preference() {
        assert_eq!(
            engine_preference_order("", Some("macos")),
            ["macos", "espeak", "piper", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("espeak", Some("macos")),
            ["espeak", "macos", "piper", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("piper", Some("macos")),
            ["piper", "espeak", "macos", "rhvoice", "flite", "rutts"]
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
            ["espeak", "piper", "rhvoice", "flite", "rutts"]
        );
        assert_eq!(
            engine_preference_order("piper", None),
            ["piper", "espeak", "rhvoice", "flite", "rutts"]
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
