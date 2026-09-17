use super::owned;
use anyhow::{Context, Result};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::helper_engine::{HelperEngineConfig, HelperTtsEngine};
use omnivox_tts::voice_library::{HostPlatform, ProviderOverrides, RuntimeLibrary};
use omnivox_tts::{SynthesisRequest, TtsEngine, TtsSettings};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod evidence;
mod managed;
mod manager;

fn host() -> HostPlatform {
    if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Posix
    }
}
struct Options {
    path: PathBuf,
    helpers: BTreeMap<String, PathBuf>,
    timeout: Duration,
    memory: usize,
    report: Option<PathBuf>,
    check_report: Option<PathBuf>,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        anyhow::ensure!(
            args.len() >= 2 && args[0] == "--validate-voice-library",
            "expected --validate-voice-library PATH"
        );
        anyhow::ensure!(!args[1].is_empty(), "voice-library path must not be empty");
        let mut options = Self {
            path: PathBuf::from(&args[1]),
            helpers: BTreeMap::new(),
            timeout: Duration::from_secs(60),
            memory: 4096usize * 1024 * 1024,
            report: None,
            check_report: None,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut remaining = args[2..].chunks_exact(2);
        for pair in &mut remaining {
            let [flag, value] = pair else { unreachable!() };
            anyhow::ensure!(
                seen.insert(flag) && !value.is_empty(),
                "duplicate or empty validation option: {flag}"
            );
            match flag.as_str() {
                "--validation-report" => options.report = Some(value.into()),
                "--check-validation-report" => options.check_report = Some(value.into()),
                "--piper-helper" | "--flite-helper" | "--mbrola-helper" | "--rhvoice-helper" => {
                    let engine = flag
                        .strip_prefix("--")
                        .unwrap()
                        .strip_suffix("-helper")
                        .unwrap();
                    let path = PathBuf::from(value)
                        .canonicalize()
                        .context("could not locate validation helper")?;
                    anyhow::ensure!(path.is_file(), "validation helper must be a regular file");
                    options.helpers.insert(engine.into(), path);
                }
                "--validation-timeout-seconds" => {
                    let seconds: u64 = value.parse().context("invalid validation timeout")?;
                    anyhow::ensure!(
                        (1..=600).contains(&seconds),
                        "validation timeout must be 1–600 seconds"
                    );
                    options.timeout = Duration::from_secs(seconds);
                }
                "--validation-memory-mib" => {
                    let mib: usize = value.parse().context("invalid validation memory limit")?;
                    anyhow::ensure!(
                        (256..=65536).contains(&mib),
                        "validation memory must be 256–65536 MiB"
                    );
                    options.memory = mib
                        .checked_mul(1024 * 1024)
                        .context("validation memory limit is too large for this host")?;
                }
                _ => anyhow::bail!("unknown validation option: {flag}"),
            }
        }
        anyhow::ensure!(
            remaining.remainder().is_empty(),
            "validation option requires a value"
        );
        anyhow::ensure!(
            options.report.is_none() || options.check_report.is_none(),
            "saving and comparing validation evidence are separate operations"
        );
        Ok(options)
    }
}

pub fn run(args: &[String]) -> Result<()> {
    if args
        .first()
        .is_some_and(|arg| arg == "--run-voice-validation-operation")
    {
        return manager::run(args);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--internal-voice-validation-supervisor")
    {
        return managed::run(args);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--internal-voice-validation-snapshot")
    {
        return evidence::worker(args);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--internal-voice-validation-worker")
    {
        return worker(args);
    }
    let options = Options::parse(args)?;
    if options.report.is_some() || options.check_report.is_some() {
        evidence::check_environment()?;
    }
    if let Some(path) = &options.report {
        evidence::check_destination(path)?;
    }
    owned::initialize()?;
    let cancelled = owned::cancellation_input()?;
    let library = RuntimeLibrary::read(File::open(&options.path)?, host())?;
    let report = validate(&options, &library, &cancelled, &mut owned::Unrecorded)?;
    if let (Some(path), Some(bytes)) = (&options.report, report) {
        evidence::publish(path, &bytes, &cancelled)?;
        println!("Saved validation evidence to {}", path.display());
    }
    Ok(())
}

fn validate(
    options: &Options,
    library: &RuntimeLibrary,
    cancelled: &AtomicBool,
    recorder: &mut dyn owned::Recorder,
) -> Result<Option<Vec<u8>>> {
    let targets = library.validation_targets();
    for target in &targets {
        anyhow::ensure!(
            options.helpers.contains_key(&target.engine_id),
            "specify --{}-helper for this validation",
            target.engine_id
        );
    }
    // Confirm all inputs and helpers before creating scratch or loading anything.
    let mut scratch = Scratch::new()?;
    let before = if options.report.is_some() || options.check_report.is_some() {
        Some(evidence::observe(
            options,
            library,
            &mut scratch,
            cancelled,
            "before",
            recorder,
        )?)
    } else {
        None
    };
    if let Some(path) = &options.check_report {
        let report =
            omnivox_tts::voice_library::evidence::ValidationEvidence::read(File::open(path)?)?;
        anyhow::ensure!(
            report.matches(before.as_ref().unwrap()),
            "saved validation observations differ from current inputs; run native validation again"
        );
        anyhow::ensure!(
            !cancelled.load(Ordering::Acquire),
            "voice validation cancelled"
        );
        scratch.remove()?;
        println!("Saved validation observations match; native validation and activation checks are still required");
        return Ok(None);
    }
    eprintln!("Validating {} native loads, one at a time; {} MiB budget, {} seconds per load. No audio playback.",
        targets.len(), options.memory / (1024 * 1024), options.timeout.as_secs());
    #[cfg(target_os = "macos")]
    eprintln!("macOS samples total process-group memory footprint; brief spikes may exceed the budget between samples.");
    for (index, target) in targets.iter().enumerate() {
        anyhow::ensure!(
            !cancelled.load(Ordering::Acquire),
            "voice validation cancelled"
        );
        let unit = library.validation_unit(target)?;
        let path = scratch.write(index, unit.source_bytes())?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--internal-voice-validation-worker")
            .arg(&path)
            .arg(unit.sha256())
            .arg(&target.engine_id)
            .arg(&options.helpers[&target.engine_id]);
        // The exact helper and projected library are explicit. Model-file
        // overrides cannot turn this into validation of unrelated legacy data.
        command
            .env_remove("OMNIVOX_VOICE_LIBRARY")
            .env_remove("OMNIVOX_PIPER_MODEL")
            .env_remove("OMNIVOX_FLITE_VOICES")
            .env_remove("OMNIVOX_REMOTE_WORKER");
        owned::probe(
            &mut command,
            options.timeout,
            options.memory,
            cancelled,
            recorder,
        )
        .with_context(|| {
            format!(
                "{} validation failed; scratch retained at {}",
                target.voice_id,
                scratch.path.display()
            )
        })?;
        anyhow::ensure!(
            !cancelled.load(Ordering::Acquire),
            "voice validation cancelled"
        );
        println!("Validated {} {}", target.engine_id, target.voice_id);
    }
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "voice validation cancelled"
    );
    let report = if let Some(before) = before {
        let after =
            evidence::observe(options, library, &mut scratch, cancelled, "after", recorder)?;
        anyhow::ensure!(
            before == after,
            "validation inputs changed; no report saved"
        );
        Some(
            omnivox_tts::voice_library::evidence::ValidationEvidence::after_success(
                after,
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            )
            .to_bytes()?,
        )
    } else {
        None
    };
    scratch.remove()?;
    println!(
        "Validated generation {}: {} native loads; cleanup confirmed",
        library.sha256(),
        targets.len()
    );
    Ok(report)
}

fn worker(args: &[String]) -> Result<()> {
    anyhow::ensure!(args.len() == 5, "invalid internal validation arguments");
    owned::worker_gate()?;
    let library = RuntimeLibrary::read_expected(File::open(&args[1])?, host(), Some(&args[2]))?;
    let targets = library.validation_targets();
    anyhow::ensure!(
        targets.len() == 1 && targets[0].engine_id == args[3],
        "validation worker requires exactly one native load"
    );
    library.verify_assets(ProviderOverrides::default())?;
    let voices: Vec<PhysicalVoiceId> = match args[3].as_str() {
        "piper" => library.document().piper.as_ref().unwrap().models[0]
            .voices
            .iter()
            .map(|voice| PhysicalVoiceId::new("piper", &voice.physical_id))
            .collect(),
        "flite" | "mbrola" | "rhvoice" => targets,
        _ => anyhow::bail!("unsupported validation engine"),
    };
    let mut config = HelperEngineConfig::new(&args[3], &args[4]);
    config.arguments = vec![
        "--voice-library".into(),
        args[1].clone().into(),
        "--voice-library-sha256".into(),
        args[2].clone().into(),
    ];
    // The outer process deadline covers verification, native startup, all
    // speakers, synthesis and retirement, even when a pipe or native call hangs.
    config.startup_timeout = Duration::from_secs(600);
    config.synthesis_idle_timeout = Duration::from_secs(600);
    let engine = HelperTtsEngine::new(config)?;
    let descriptor = engine.descriptor();
    anyhow::ensure!(
        descriptor.voices.len() == voices.len()
            && descriptor
                .voices
                .iter()
                .all(|voice| voices.contains(&voice.id) && voice.availability.is_available()),
        "helper returned a different validation inventory"
    );
    for voice in voices {
        let request = SynthesisRequest::new("a", TtsSettings::default())
            .with_route("validation", voice.clone());
        let result = engine.synthesize(&request)?;
        anyhow::ensure!(
            result.actual_voice.as_ref() == Some(&voice) && !result.audio.samples.is_empty(),
            "native voice validation did not produce PCM with the requested identity"
        );
    }
    drop(engine);
    std::io::stdout().write_all(owned::RECEIPT)?;
    Ok(())
}

struct Scratch {
    path: PathBuf,
    files: Vec<PathBuf>,
}
impl Scratch {
    fn new() -> Result<Self> {
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        for attempt in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "omnivox-voice-validation-{}-{time}-{attempt}",
                std::process::id()
            ));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path: path.canonicalize()?,
                        files: Vec::new(),
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        anyhow::bail!("could not allocate validation scratch directory")
    }
    fn write(&mut self, index: usize, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.path.join(format!("unit-{index}.json"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        self.files.push(path.clone());
        file.write_all(bytes)?;
        Ok(path)
    }
    fn remove(&mut self) -> Result<()> {
        for path in &self.files {
            fs::remove_file(path)?;
        }
        fs::remove_dir(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_options_require_bounded_unambiguous_limits() {
        let parse =
            |args: &[&str]| Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let valid = parse(&[
            "--validate-voice-library",
            "generation.json",
            "--validation-memory-mib",
            "512",
            "--validation-timeout-seconds",
            "3",
        ])
        .unwrap();
        assert_eq!(valid.memory, 512 * 1024 * 1024);
        assert_eq!(valid.timeout, Duration::from_secs(3));
        for tail in [
            vec!["--validation-memory-mib", "0"],
            vec!["--validation-timeout-seconds", "601"],
            vec!["--validation-memory-mib", "65537"],
            vec!["--validation-timeout-seconds"],
            vec!["--unknown", "1"],
            vec![
                "--validation-timeout-seconds",
                "1",
                "--validation-timeout-seconds",
                "2",
            ],
        ] {
            let mut args = vec!["--validate-voice-library", "generation.json"];
            args.extend(tail);
            assert!(parse(&args).is_err());
        }
    }
}
