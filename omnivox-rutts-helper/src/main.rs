use std::sync::Arc;

use omnivox_helper_host::run_stdio;
use omnivox_rutts_helper::RuttsTtsEngine;
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(RuttsTtsEngine::new());
    run_stdio(engine, "Omnivox RuTTS helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
