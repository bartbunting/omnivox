use std::sync::Arc;

use omnivox_flite_helper::FliteTtsEngine;
use omnivox_helper_host::run_stdio;
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(FliteTtsEngine::from_environment()?);
    run_stdio(engine, "Omnivox Flite helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
