use std::path::PathBuf;
use std::sync::Arc;

use omnivox_helper_host::run_stdio;
use omnivox_tts::piper::PiperTtsEngine;
use omnivox_tts::voice_library::{HostPlatform, RuntimeLibrary};
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(match parse_source(std::env::args_os().skip(1))? {
        Source::Model(path) => PiperTtsEngine::new(path)?,
        Source::Library(path) => {
            let host = if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            };
            let library = RuntimeLibrary::read(std::fs::File::open(path)?, host)?;
            PiperTtsEngine::from_library(&library)?
        }
    });
    run_stdio(engine, "Omnivox Piper helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Source {
    Model(PathBuf),
    Library(PathBuf),
}

fn parse_source(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Source, String> {
    let mut arguments = arguments.into_iter();
    let mut source = None;
    while let Some(argument) = arguments.next() {
        if argument == "--model" || argument == "--voice-library" {
            let value = arguments
                .next()
                .ok_or_else(|| format!("{argument:?} requires a path"))?;
            if value.is_empty() {
                return Err(format!("{argument:?} requires a non-empty path"));
            }
            let candidate = if argument == "--model" {
                Source::Model(value.into())
            } else {
                Source::Library(value.into())
            };
            match (&source, &candidate) {
                // Preserve the existing last --model wins behavior.
                (None, _) | (Some(Source::Model(_)), Source::Model(_)) => source = Some(candidate),
                _ => return Err(
                    "--model and --voice-library are mutually exclusive; specify one library once"
                        .to_owned(),
                ),
            }
        } else {
            return Err(format!("unknown Piper helper argument: {:?}", argument));
        }
    }
    source.ok_or_else(|| {
        "usage: omnivox-piper-helper --model MODEL.onnx | --voice-library GENERATION.json"
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Source, String> {
        parse_source(args.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn preserves_legacy_selection_and_rejects_ambiguous_library_startup() {
        assert_eq!(
            parse(&["--model", "first.onnx", "--model", "second.onnx"]).unwrap(),
            Source::Model("second.onnx".into())
        );
        assert_eq!(
            parse(&["--voice-library", "generation.json"]).unwrap(),
            Source::Library("generation.json".into())
        );
        for args in [
            vec![],
            vec!["--model"],
            vec!["--voice-library", ""],
            vec!["--model", "a", "--voice-library", "b"],
            vec!["--voice-library", "a", "--model", "b"],
            vec!["--voice-library", "a", "--voice-library", "b"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
