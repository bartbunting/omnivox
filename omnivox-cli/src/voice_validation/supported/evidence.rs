//! Optional observations for the development validator. No index mutation or reuse.
use super::{host, owned, Options, Scratch};
use anyhow::{Context, Result};
use omnivox_tts::voice_library::evidence::EvidenceSnapshot;
use omnivox_tts::voice_library::RuntimeLibrary;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn unsupported_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.starts_with("LD_")
        || name.starts_with("DYLD_")
        || name.starts_with("_RLD")
        || matches!(
            name.as_str(),
            "LIBPATH" | "SHLIB_PATH" | "OMNIVOX_PIPER_ESPEAK_DATA" | "ESPEAK_NG_DATA"
        )
}

pub(super) fn check_environment() -> Result<()> {
    for (name, value) in std::env::vars_os() {
        anyhow::ensure!(value.is_empty() || !unsupported_variable(&name.to_string_lossy()),
            "saved validation evidence does not support the runtime override {}; use bundled companion data and the normal native loader",
            name.to_string_lossy());
    }
    Ok(())
}

pub(super) fn check_destination(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => anyhow::bail!(
            "validation report destination already exists: {}",
            path.display()
        ),
    }
}

pub(super) fn observe(
    options: &Options,
    library: &RuntimeLibrary,
    scratch: &mut Scratch,
    cancelled: &AtomicBool,
    phase: &str,
    recorder: &mut dyn owned::Recorder,
) -> Result<EvidenceSnapshot> {
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "voice validation cancelled"
    );
    let output = scratch.path.join(format!("{phase}-snapshot.json"));
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--internal-voice-validation-snapshot")
        .arg(&options.path)
        .arg(library.sha256())
        .arg(&output)
        .arg(std::env::current_exe()?)
        .arg(options.timeout.as_secs().to_string())
        .arg(options.memory.to_string());
    for target in library
        .validation_targets()
        .iter()
        .map(|target| &target.engine_id)
        .collect::<std::collections::BTreeSet<_>>()
    {
        command.arg(target).arg(&options.helpers[target]);
    }
    owned::probe(
        &mut command,
        options.timeout,
        options.memory,
        cancelled,
        recorder,
    )
    .with_context(|| {
        format!(
            "{phase} input observation failed; scratch retained at {}",
            scratch.path.display()
        )
    })?;
    scratch.files.push(output.clone());
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "voice validation cancelled"
    );
    EvidenceSnapshot::read(File::open(output)?).context("invalid worker input observation")
}

pub(super) fn worker(args: &[String]) -> Result<()> {
    anyhow::ensure!(
        args.len() >= 7 && args.len() <= 11 && (args.len() - 7).is_multiple_of(2),
        "invalid internal evidence arguments"
    );
    owned::worker_gate()?;
    check_environment()?;
    let library = RuntimeLibrary::read_expected(File::open(&args[1])?, host(), Some(&args[2]))?;
    let mut helpers = BTreeMap::new();
    for pair in args[7..].chunks_exact(2) {
        anyhow::ensure!(
            matches!(pair[0].as_str(), "piper" | "flite")
                && helpers
                    .insert(pair[0].clone(), PathBuf::from(&pair[1]))
                    .is_none(),
            "invalid evidence helper selection"
        );
    }
    let snapshot = EvidenceSnapshot::capture(
        &library,
        Path::new(&args[4]),
        &helpers,
        args[5].parse()?,
        args[6].parse()?,
    )?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?;
    output.write_all(&snapshot.to_bytes()?)?;
    drop(output);
    std::io::stdout().write_all(owned::RECEIPT)?;
    Ok(())
}

/// Publish an already complete record through a same-directory hard link. Link
/// creation is the commit point and never replaces an existing name. Filesystems
/// without hard links fail closed. This is not a power-loss-durable transaction.
pub(super) fn publish(path: &Path, bytes: &[u8], cancelled: &AtomicBool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    let destination = parent.join(path.file_name().context("report path needs a filename")?);
    let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    struct Pending(PathBuf);
    impl Drop for Pending {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    for attempt in 0..100 {
        let temporary = parent.join(format!(
            ".omnivox-evidence-{}-{time}-{attempt}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary);
        match file {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
            Ok(mut file) => {
                let _pending = Pending(temporary.clone());
                file.write_all(bytes)?;
                file.sync_all()?;
                drop(file);
                anyhow::ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "voice validation cancelled; no report saved"
                );
                fs::hard_link(&temporary, &destination)
                    .context("could not publish validation report without replacing a file")?;
                // Cancellation after this point cannot revoke a published report.
                // Temporary-name cleanup is best effort and never removes destination.
                return Ok(());
            }
        }
    }
    anyhow::bail!("could not allocate temporary validation report")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_preserve_existing_destinations_and_cancel_before_publication() {
        let mut scratch = Scratch::new().unwrap();
        let destination = scratch.path.join("report.json");
        let cancelled = AtomicBool::new(false);
        publish(&destination, b"first complete record", &cancelled).unwrap();
        assert!(publish(&destination, b"second record", &cancelled).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"first complete record");
        let absent = scratch.path.join("cancelled.json");
        cancelled.store(true, Ordering::Release);
        assert!(publish(&absent, b"cancelled record", &cancelled).is_err());
        assert!(!absent.exists());
        assert_eq!(fs::read_dir(&scratch.path).unwrap().count(), 1);
        scratch.files.push(destination);
        scratch.remove().unwrap();
    }

    #[test]
    fn saved_evidence_rejects_data_and_loader_overrides() {
        for variable in [
            "OMNIVOX_PIPER_ESPEAK_DATA",
            "ESPEAK_NG_DATA",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "_RLD_LIST",
            "LIBPATH",
        ] {
            assert!(unsupported_variable(variable));
        }
        assert!(!unsupported_variable("PATH"));
    }
}
