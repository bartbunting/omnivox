//! List available voices on the system

use omnivox_tts::espeak::EspeakTtsEngine;
#[cfg(target_os = "macos")]
use omnivox_tts::macos::MacOsTtsEngine;
#[cfg(target_os = "windows")]
use omnivox_tts::windows::WindowsTtsEngine;
use omnivox_tts::TtsEngine;

fn main() {
    let engine: Box<dyn TtsEngine> = {
        #[cfg(target_os = "macos")]
        {
            match MacOsTtsEngine::new() {
                Ok(e) => Box::new(e),
                Err(_) => Box::new(
                    EspeakTtsEngine::new().expect("Failed to create TTS engine"),
                ),
            }
        }
        #[cfg(target_os = "windows")]
        {
            match WindowsTtsEngine::new() {
                Ok(e) => Box::new(e),
                Err(_) => Box::new(
                    EspeakTtsEngine::new().expect("Failed to create TTS engine"),
                ),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Box::new(EspeakTtsEngine::new().expect("Failed to create TTS engine"))
        }
    };

    let voices = engine.available_voices();

    println!("Found {} voices:\n", voices.len());

    // Group by language
    let mut by_lang: std::collections::BTreeMap<String, Vec<_>> = std::collections::BTreeMap::new();

    for voice in voices {
        by_lang.entry(voice.language.clone()).or_default().push(voice);
    }

    for (lang, voices) in by_lang {
        println!("{} ({} voices):", lang, voices.len());
        for voice in voices {
            println!("  {:?} - {}", voice.quality, voice.name);
        }
        println!();
    }
}
