//! TTS engine selection and creation.

use anyhow::Result;
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(any(target_os = "windows", feature = "piper"))]
use omnivox_tts::helper_engine::{HelperEngineConfig, HelperTtsEngine};
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::TtsEngine;
#[cfg(any(target_os = "windows", feature = "piper"))]
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
#[cfg(any(target_os = "windows", feature = "piper"))]
use std::time::Duration;
use tracing::{info, warn};

use crate::engine_execution::{IsolatedTtsEngine, IsolationBudget};

/// Engines initialized for one server session.
pub struct CreatedEngines {
    pub preferred: Arc<dyn TtsEngine>,
    pub registry: EngineRegistry,
}

/// Create all engines that should be available to the server process.
///
/// Windows eagerly initializes WinRT and eSpeak so that the registry can expose
/// both engines. Other platforms retain the current single-engine startup until
/// their multi-engine policy is defined.
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
        let preferred = isolate_server_engine(
            create_engine(engine_name, piper_model)?,
            generation,
            isolation_budget,
        );
        let mut registry = EngineRegistry::new();
        registry.register(Arc::clone(&preferred))?;
        Ok(CreatedEngines {
            preferred,
            registry,
        })
    }
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

    if forced == "piper" {
        #[cfg(feature = "piper")]
        {
            match piper_helper_config(piper_model) {
                Ok(config) => register_optional_helper(
                    &mut registry,
                    Some(config),
                    Arc::clone(&generation),
                    Arc::clone(&isolation_budget),
                )?,
                Err(error) => warn!("Piper TTS helper not available: {error}"),
            }
        }
        #[cfg(not(feature = "piper"))]
        warn!(
            "OMNIVOX_ENGINE=piper but omnivox was built without piper support. \
             Rebuild with --features piper."
        );
    }

    let preferred = windows_preference_order(&forced)
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

#[cfg(any(target_os = "windows", feature = "piper"))]
fn helper_config(
    engine_id: &str,
    environment_variable: &str,
    adjacent_filename: &str,
) -> Option<HelperEngineConfig> {
    let explicitly_configured = std::env::var_os(environment_variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let program = match explicitly_configured {
        Some(program) => program,
        None => {
            let adjacent = std::env::current_exe()
                .ok()?
                .parent()?
                .join(adjacent_filename);
            if !adjacent.is_file() {
                return None;
            }
            adjacent
        }
    };

    let mut config = HelperEngineConfig::new(engine_id, program);
    // Version-1 native helpers capture one complete utterance before emitting
    // PCM. Allow ordinary long passages without weakening startup or control
    // request timeouts.
    config.synthesis_idle_timeout = Duration::from_secs(60);
    Some(config)
}

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn requested_engine(engine_name: &str) -> String {
    if engine_name.is_empty() {
        std::env::var("OMNIVOX_ENGINE").unwrap_or_default()
    } else {
        engine_name.to_owned()
    }
}

#[cfg(any(test, target_os = "windows"))]
fn windows_preference_order(requested: &str) -> &'static [&'static str] {
    match requested {
        "espeak" => &["espeak", "winrt"],
        "piper" => &["piper", "espeak", "winrt"],
        _ => &["winrt", "espeak"],
    }
}

/// Create a TTS engine by name, falling back through the platform default to espeak-ng.
///
/// `engine_name` may be empty (use `OMNIVOX_ENGINE` env var or platform default),
/// `"espeak"`, or `"piper"`.  `piper_model` is the path to a `.onnx` model file;
/// if `None`, `OMNIVOX_PIPER_MODEL` is consulted.
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

    if forced != "espeak" && forced != "piper" {
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
    if engine.descriptor().id != "piper" {
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
    let mut config =
        helper_config("piper", "OMNIVOX_PIPER_HELPER", &helper_filename).ok_or_else(|| {
            anyhow::anyhow!(
                "{} was not found beside Omnivox; set OMNIVOX_PIPER_HELPER",
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

/// Apply `OMNIVOX_AUDIO_TARGET` env var to the state's channel routing fields.
pub fn apply_audio_target_env(state: &mut TtsState) {
    if let Ok(target) = std::env::var("OMNIVOX_AUDIO_TARGET") {
        if let Some(channel_mode) = ChannelMode::parse(&target) {
            info!("Setting audio target from env: {}", target);
            state.speech_routing.channel_mode = channel_mode;
            state.notification_routing.channel_mode = channel_mode;
            state.tone_routing.channel_mode = channel_mode;
            state.sound_routing.channel_mode = channel_mode;
        } else {
            warn!("Invalid OMNIVOX_AUDIO_TARGET value: {}", target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::windows_preference_order;

    #[test]
    fn windows_defaults_to_winrt_with_espeak_fallback() {
        assert_eq!(windows_preference_order(""), &["winrt", "espeak"]);
        assert_eq!(windows_preference_order("native"), &["winrt", "espeak"]);
    }

    #[test]
    fn windows_honours_explicit_engine_preferences() {
        assert_eq!(windows_preference_order("espeak"), &["espeak", "winrt"]);
        assert_eq!(
            windows_preference_order("piper"),
            &["piper", "espeak", "winrt"]
        );
    }
}
