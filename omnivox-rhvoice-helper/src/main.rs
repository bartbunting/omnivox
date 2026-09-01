use std::sync::Arc;

use omnivox_helper_host::run_stdio;
use omnivox_rhvoice_helper::RhVoiceTtsEngine;
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(RhVoiceTtsEngine::from_environment());
    run_stdio(engine, "Omnivox RHVoice helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
