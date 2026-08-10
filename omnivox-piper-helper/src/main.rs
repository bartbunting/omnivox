use std::path::PathBuf;
use std::sync::Arc;

use omnivox_piper_helper::run_stdio;
use omnivox_tts::piper::PiperTtsEngine;
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = parse_model_path()?;
    let engine: Arc<dyn TtsEngine> = Arc::new(PiperTtsEngine::new(model)?);
    run_stdio(engine, "Omnivox Piper helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn parse_model_path() -> Result<PathBuf, String> {
    let mut arguments = std::env::args_os().skip(1);
    let mut model = None;
    while let Some(argument) = arguments.next() {
        if argument == "--model" {
            let value = arguments
                .next()
                .ok_or_else(|| "--model requires a path".to_owned())?;
            if value.is_empty() {
                return Err("--model requires a non-empty path".to_owned());
            }
            model = Some(PathBuf::from(value));
        } else {
            return Err(format!("unknown Piper helper argument: {:?}", argument));
        }
    }
    model.ok_or_else(|| "usage: omnivox-piper-helper --model MODEL.onnx".to_owned())
}
