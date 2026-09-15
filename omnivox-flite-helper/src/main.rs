use std::sync::Arc;

use omnivox_flite_helper::FliteTtsEngine;
use omnivox_helper_host::run_stdio;
use omnivox_tts::voice_library::{HostPlatform, RuntimeLibrary};
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(match library_path(std::env::args_os().skip(1))? {
        None => FliteTtsEngine::from_environment()?,
        Some(path) => {
            let host = if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            };
            let library = RuntimeLibrary::read(std::fs::File::open(path)?, host)?;
            FliteTtsEngine::from_library(&library)?
        }
    });
    run_stdio(engine, "Omnivox Flite helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn library_path(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<std::path::PathBuf>, String> {
    let mut arguments = arguments.into_iter();
    let Some(flag) = arguments.next() else {
        return Ok(None);
    };
    if flag != "--voice-library" {
        return Err(format!("unknown Flite helper argument: {flag:?}"));
    }
    let path = arguments
        .next()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "--voice-library requires a non-empty path".to_owned())?;
    if arguments.next().is_some() {
        return Err("specify exactly one --voice-library path".to_owned());
    }
    Ok(Some(path.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_requires_one_unambiguous_library_or_legacy_environment() {
        let parse = |args: &[&str]| library_path(args.iter().map(std::ffi::OsString::from));
        assert_eq!(parse(&[]).unwrap(), None);
        assert_eq!(
            parse(&["--voice-library", "generation.json"]).unwrap(),
            Some("generation.json".into())
        );
        for args in [
            vec!["--voice-library"],
            vec!["--voice-library", ""],
            vec!["--voice-library", "a", "--voice-library", "b"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
