//! Development installed-voice storage and explicit activation preparation.
use anyhow::{Context, Result};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::voice_library::installation::Profile;
use std::io::Write;
use std::path::Path;

pub fn requested(args: &[String]) -> bool {
    args.first().is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "--initialize-voice-library"
                | "--inspect-voice-library"
                | "--import-validated-voice"
                | "--set-library-voice-enabled"
                | "--stage-voice-library-activation"
                | "--inspect-voice-library-activation"
        )
    })
}

pub fn run(args: &[String]) -> Result<()> {
    let command = args
        .first()
        .context("missing voice-library command")?
        .as_str();
    let count = match command {
        "--initialize-voice-library" | "--inspect-voice-library-activation" => 4,
        "--inspect-voice-library" => 3,
        "--import-validated-voice" | "--set-library-voice-enabled" => 8,
        "--stage-voice-library-activation" => 6,
        _ => anyhow::bail!("unknown voice-library command"),
    };
    anyhow::ensure!(
        args.len() == count,
        "{command}: wrong argument count; see docs/VOICE-INSTALLATION.md"
    );
    let root = Path::new(&args[1]);
    if command == "--initialize-voice-library" {
        let profile = Profile::initialize(root, &args[2], &args[3])?;
        println!(
            "Initialized empty voice library; index SHA-256 {}",
            profile.index_sha256()
        );
        return Ok(());
    }
    let mut profile = Profile::open(root, &args[2])?;
    match command {
        "--inspect-voice-library" => {
            std::io::stdout().write_all(profile.index().source_bytes())?;
        }
        "--import-validated-voice" => {
            profile.import_validated(&args[3], &args[4], &args[5], &args[6], &args[7])?;
            println!(
                "Imported voices disabled; original files retained; index SHA-256 {}",
                profile.index_sha256()
            );
        }
        "--set-library-voice-enabled" => {
            let enabled = match args[5].as_str() {
                "true" => true,
                "false" => false,
                _ => anyhow::bail!("enablement must be true or false"),
            };
            profile.set_enabled(
                &PhysicalVoiceId::new(&args[3], &args[4]),
                enabled,
                &args[6],
                &args[7],
            )?;
            println!(
                "Saved desired enablement; activation remains pending; index SHA-256 {}",
                profile.index_sha256()
            );
        }
        "--stage-voice-library-activation" => {
            let (piper, flite, mbrola) = match args[4].as_str() {
                "piper" => (true, false, false),
                "flite" => (false, true, false),
                "mbrola" => (false, false, true),
                "both" => (true, true, false),
                "all" => (true, true, true),
                _ => anyhow::bail!("managed providers must be piper, flite, mbrola, both, or all"),
            };
            let candidate = profile.stage_activation(&args[3], piper, flite, mbrola, &args[5])?;
            std::io::stdout().write_all(&candidate.to_bytes()?)?;
            println!();
        }
        "--inspect-voice-library-activation" => {
            let candidate = profile.activation_candidate(&args[3])?;
            std::io::stdout().write_all(&candidate.to_bytes()?)?;
            println!();
        }
        _ => unreachable!(),
    }
    Ok(())
}
