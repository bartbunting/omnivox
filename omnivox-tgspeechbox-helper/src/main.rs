use std::sync::Arc;

use omnivox_helper_host::run_stdio;
use omnivox_tgspeechbox_helper::TgSpeechBoxTtsEngine;
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(TgSpeechBoxTtsEngine::from_environment()?);
    run_stdio(
        engine,
        "Omnivox TGSpeechBox helper",
        env!("CARGO_PKG_VERSION"),
    )?;
    Ok(())
}
