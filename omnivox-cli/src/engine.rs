//! TTS engine selection and creation.

use anyhow::Result;
use omnivox_core::state::ChannelMode;
use omnivox_core::TtsState;
use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(feature = "piper")]
use omnivox_tts::piper::PiperTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::TtsEngine;
use std::sync::Arc;
use tracing::{info, warn};

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
            let model = _piper_model
                .map(str::to_string)
                .or_else(|| std::env::var("OMNIVOX_PIPER_MODEL").ok());

            match model {
                Some(ref path) => match PiperTtsEngine::new(path) {
                    Ok(engine) => {
                        info!("Using piper neural TTS engine: {}", path);
                        return Ok(Arc::new(engine));
                    }
                    Err(e) => warn!("Piper TTS not available: {}, falling back to espeak-ng", e),
                },
                None => warn!(
                    "OMNIVOX_ENGINE=piper but no model path given. \
                     Set OMNIVOX_PIPER_MODEL or use --piper-model. \
                     Falling back to espeak-ng."
                ),
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
            Err(e) => warn!("Windows WinRT not available: {}, falling back to espeak-ng", e),
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

/// Human-readable name of the platform-native TTS backend.
pub fn native_engine_name() -> &'static str {
    #[cfg(target_os = "macos")]
    { "macos (AVSpeechSynthesizer)" }
    #[cfg(target_os = "windows")]
    { "winrt (Windows SpeechSynthesizer)" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { "none (espeak-ng is the only backend)" }
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
