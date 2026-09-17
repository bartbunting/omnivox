use std::sync::Arc;

use omnivox_helper_host::run_stdio;
use omnivox_rhvoice_helper::RhVoiceTtsEngine;
use omnivox_tts::voice_library::{HostPlatform, RuntimeLibrary};
use omnivox_tts::TtsEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine: Arc<dyn TtsEngine> = Arc::new(match library_path(std::env::args_os().skip(1))? {
        None => RhVoiceTtsEngine::from_environment(),
        Some((path, expected)) => {
            let host = if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            };
            let library = RuntimeLibrary::read_expected(
                std::fs::File::open(path)?,
                host,
                expected.as_deref(),
            )?;
            RhVoiceTtsEngine::from_library(&library)?
        }
    });
    run_stdio(engine, "Omnivox RHVoice helper", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

fn library_path(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Option<(std::path::PathBuf, Option<String>)>, String> {
    let mut arguments = arguments.into_iter();
    let Some(flag) = arguments.next() else {
        return Ok(None);
    };
    if flag != "--voice-library" {
        return Err(format!("unknown RHVoice helper argument: {flag:?}"));
    }
    let path = arguments
        .next()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "--voice-library requires a non-empty path".to_owned())?;
    let expected = match arguments.next() {
        None => None,
        Some(flag) if flag == "--voice-library-sha256" => Some(
            arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
                .ok_or("--voice-library-sha256 requires a digest")?,
        ),
        _ => return Err("specify exactly one --voice-library path".to_owned()),
    };
    if arguments.next().is_some() {
        return Err("unexpected RHVoice helper argument".to_owned());
    }
    Ok(Some((path.into(), expected)))
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
            Some(("generation.json".into(), None))
        );
        for args in [
            vec!["--voice-library"],
            vec!["--voice-library", ""],
            vec!["--voice-library", "a", "--voice-library", "b"],
            vec!["--unknown"],
            vec!["--voice-library-sha256", "abc"],
            vec!["--voice-library", "a", "--voice-library-sha256"],
            vec!["--voice-library", "a", "--voice-library-sha256", ""],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
