//! Explicit local downloads. Helpers never receive catalogue or network access.
use anyhow::{Context, Result};
use omnivox_tts::voice_library::acquisition::{self, Acquisition, Progress};
use omnivox_tts::voice_library::catalogue::{Catalogue, DownloadFile};
use omnivox_tts::voice_library::local::{Host, Reply, Request};
use omnivox_tts::voice_library::operations::{Operation, ValidationPlan, ValidationPlanDocument};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn send(id: u64, reply: Reply) -> Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(reply.line(id)?.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

pub fn run(host: Host) -> Result<()> {
    let mut input = BufReader::new(io::stdin());
    let bytes = crate::voice_local::line(&mut input)?.context("no acquisition request")?;
    let request = Request::parse(&bytes)?;
    let setup = (|| -> Result<_> {
        anyhow::ensure!(
            request.command == "acquire",
            "expected a voice acquisition request"
        );
        let catalogue = Catalogue::parse(request.plan_json.as_bytes())?;
        let operation = Acquisition::prepare(&host, &catalogue, &request.voice)?;
        Ok((catalogue, operation))
    })();
    let (catalogue, mut operation) = match setup {
        Ok(value) => value,
        Err(error) => {
            send(
                request.request_id,
                Reply::Error {
                    message: format!("{error:#}"),
                },
            )?;
            return Ok(());
        }
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    std::thread::Builder::new()
        .name("voice-download-cancellation".into())
        .spawn(move || {
            let mut byte = [0];
            loop {
                match input.read(&mut byte) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => {
                        signal.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        })?;
    let entry = catalogue.entry(&request.voice)?;
    let mut progress = Progress {
        operation_id: operation.plan.operation_id.clone(),
        state: "preparing".into(),
        downloaded_bytes: 0,
        total_bytes: entry.total_bytes(),
        terminal: false,
        detail: format!("Preparing {}", entry.name),
    };
    let result = execute(
        &host,
        &catalogue,
        &mut operation,
        &cancelled,
        &mut progress,
        &mut |event| {
            send(
                request.request_id,
                Reply::Acquisition {
                    progress: event.clone(),
                },
            )
        },
    );
    progress.terminal = true;
    match result {
        Ok(()) => {
            progress.state = "installed-disabled".into();
            progress.detail =
                "Voice installed disabled; enable it and review Apply when ready".into();
        }
        Err(error) => {
            progress.state = if cancelled.load(Ordering::Acquire) {
                "stopped-needs-inspection"
            } else {
                "failed"
            }
            .into();
            // No resume, deletion, success or cleanup claim follows from an error.
            let detail = format!("{error:#}; operation files retained for inspection");
            progress.detail = detail
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            while progress.detail.len() > 1500 {
                progress.detail.pop();
            }
        }
    }
    operation.record(&progress)?;
    send(request.request_id, Reply::Acquisition { progress })
}

fn checkpoint(cancelled: &AtomicBool) -> Result<()> {
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        "voice installation cancelled"
    );
    Ok(())
}

fn execute(
    host: &Host,
    catalogue: &Catalogue,
    operation: &mut Acquisition,
    cancelled: &Arc<AtomicBool>,
    progress: &mut Progress,
    notify: &mut impl FnMut(&Progress) -> Result<()>,
) -> Result<()> {
    let entry = catalogue.entry(&operation.plan.entry_id)?;
    // Resolve only the selected bundled companion; no discovery of live engines.
    let executable = std::env::current_exe()?.canonicalize()?;
    let engine = entry.engine_id();
    let helper = executable
        .parent()
        .context("speech executable has no directory")?
        .join(engine)
        .join(format!(
            "omnivox-{engine}-helper{}",
            std::env::consts::EXE_SUFFIX
        ));
    anyhow::ensure!(
        helper.is_file(),
        "The {engine} companion is missing from this speech runtime"
    );
    checkpoint(cancelled)?;
    operation.record(progress)?;
    notify(progress)?;
    let started = Instant::now();
    for file in &entry.files {
        checkpoint(cancelled)?;
        progress.state = "downloading".into();
        progress.detail = format!("Downloading {}", file.role);
        operation.record(progress)?;
        notify(progress)?;
        let partial = operation
            .staging
            .join(format!("{}.partial", file.filename()?));
        let mut destination = acquisition::new_download(&partial)?;
        let response = download(file.clone(), cancelled.clone())?;
        let base = progress.downloaded_bytes;
        let mut last = Instant::now();
        let mut last_bytes = 0;
        copy_verified(response, &mut destination, file, cancelled, |count| {
            anyhow::ensure!(
                started.elapsed() < Duration::from_secs(900),
                "voice download deadline exceeded"
            );
            progress.downloaded_bytes = base + count;
            if count - last_bytes >= 2 * 1024 * 1024 || last.elapsed() >= Duration::from_secs(1) {
                operation.record(progress)?;
                notify(progress)?;
                last = Instant::now();
                last_bytes = count;
            }
            Ok(())
        })?;
        destination.sync_all()?;
        drop(destination);
        checkpoint(cancelled)?;
        std::fs::rename(&partial, operation.staging.join(file.filename()?))?;
    }
    checkpoint(cancelled)?;
    operation.place_files(catalogue)?;
    progress.state = "validating".into();
    progress.detail = "Checking the downloaded voice in an isolated native helper".into();
    operation.record(progress)?;
    notify(progress)?;
    let library = entry.generation(
        &host.target_id,
        &host.profile_id,
        &operation.plan.generation_id,
        &operation.package,
    )?;
    let plan = ValidationPlanDocument {
        schema_version: 1,
        operation_kind: "native_validation".into(),
        operation_id: operation.plan.operation_id.clone(),
        platform: std::env::consts::OS.into(),
        generation_json: String::from_utf8(library.source_bytes().to_vec())?,
        validator_path: omnivox_tts::voice_library::catalogue::metadata_path(&executable)?,
        helpers: BTreeMap::from([(
            engine.into(),
            omnivox_tts::voice_library::catalogue::metadata_path(&helper)?,
        )]),
        timeout_seconds: 120,
        memory_bytes: 4096 * 1024 * 1024,
        runtime_policy: "bundled-companions-v1".into(),
    };
    drop(Operation::create(
        &host.root.join("operations"),
        ValidationPlan::parse(&serde_json::to_vec(&plan)?)?,
    )?);
    validate(&executable, host, operation, cancelled)?;
    checkpoint(cancelled)?;
    progress.state = "installing".into();
    progress.detail = "Adding the verified voice to the installed library, disabled".into();
    operation.record(progress)?;
    notify(progress)?;
    checkpoint(cancelled)?;
    host.profile()?.install_catalogue(
        &operation.plan.operation_id,
        catalogue,
        &entry.id,
        &operation.plan.package_id,
        &operation.plan.revision_id,
        &operation.plan.expected_index_sha256,
    )?;
    Ok(())
}

// A bounded network-only thread lets stdin cancellation interrupt a stalled
// read without killing the native validation cleanup owner. It never opens
// voice storage or loads a helper. Dropping the receiver stops delivery; this
// one-operation CLI exits on cancellation, closing any outstanding socket.
struct DownloadReader {
    receiver: mpsc::Receiver<io::Result<Vec<u8>>>,
    current: io::Cursor<Vec<u8>>,
    cancelled: Arc<AtomicBool>,
    ended: bool,
}
impl Read for DownloadReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(io::Error::other("voice download cancelled"));
            }
            let count = self.current.read(output)?;
            if count > 0 || self.ended {
                return Ok(count);
            }
            match self.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(bytes)) => {
                    self.ended = bytes.is_empty();
                    self.current = io::Cursor::new(bytes);
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other(
                        "voice download connection ended unexpectedly",
                    ))
                }
            }
        }
    }
}
fn download(file: DownloadFile, cancelled: Arc<AtomicBool>) -> Result<DownloadReader> {
    let (sender, receiver) = mpsc::sync_channel(2);
    std::thread::Builder::new()
        .name("voice-download-network".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let config = ureq::Agent::config_builder()
                    .https_only(file.url.starts_with("https://"))
                    .max_redirects(5)
                    .timeout_global(Some(Duration::from_secs(900)))
                    .timeout_connect(Some(Duration::from_secs(15)))
                    .timeout_resolve(Some(Duration::from_secs(15)))
                    .timeout_recv_response(Some(Duration::from_secs(20)))
                    .timeout_recv_body(Some(Duration::from_secs(900)))
                    .user_agent("Omnivox voice installer/1")
                    .build();
                let agent = config.new_agent();
                let mut response = agent
                    .get(&file.url)
                    .header("Accept-Encoding", "identity")
                    .call()
                    .with_context(|| format!("could not download {}", file.role))?;
                anyhow::ensure!(
                    response.status().as_u16() == 200,
                    "download requires a complete HTTP 200 response"
                );
                if let Some(length) = response.headers().get("content-length") {
                    let declared: u64 = length.to_str()?.parse()?;
                    anyhow::ensure!(
                        declared == file.bytes,
                        "download size differs from the reviewed catalogue"
                    );
                }
                let mut body = response.body_mut().as_reader();
                loop {
                    let mut bytes = vec![0; 64 * 1024];
                    let count = body.read(&mut bytes)?;
                    bytes.truncate(count);
                    if sender.send(Ok(bytes)).is_err() || count == 0 {
                        return Ok(());
                    }
                }
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(io::Error::other(format!("{error:#}"))));
            }
        })?;
    Ok(DownloadReader {
        receiver,
        current: io::Cursor::new(Vec::new()),
        cancelled,
        ended: false,
    })
}

fn copy_verified(
    mut input: impl Read,
    output: &mut impl Write,
    file: &DownloadFile,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64) -> Result<()>,
) -> Result<()> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        checkpoint(cancelled)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        anyhow::ensure!(bytes <= file.bytes, "download exceeds the reviewed size");
        output.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
        progress(bytes)?;
    }
    checkpoint(cancelled)?;
    anyhow::ensure!(
        bytes == file.bytes,
        "download ended before the reviewed size"
    );
    anyhow::ensure!(
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            == file.sha256,
        "download checksum differs from the reviewed catalogue"
    );
    Ok(())
}

fn validate(
    executable: &Path,
    host: &Host,
    operation: &Acquisition,
    cancelled: &AtomicBool,
) -> Result<()> {
    let diagnostics = acquisition::new_download(&operation.directory.join("validation.log"))?;
    let mut child = Command::new(executable)
        .args([
            "--run-voice-validation-operation",
            &host.root.to_string_lossy(),
            &host.profile_id,
            &operation.plan.operation_id,
        ])
        // This disposable check uses the bundled companion's own eSpeak data.
        .env_remove("ESPEAK_NG_DATA")
        .stdin(Stdio::piped())
        .stdout(diagnostics.try_clone()?)
        .stderr(diagnostics)
        .spawn()?;
    loop {
        if cancelled.load(Ordering::Acquire) {
            child.stdin.take();
        }
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success(),
                "native validation failed; see the retained validation.log"
            );
            return Ok(());
        }
        // The existing supervisor owns deadlines, native trees and cleanup.
        // Client cancellation closes its pipe; never kill that cleanup owner.
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_interrupts_a_stalled_network_reader() {
        let (_sender, receiver) = mpsc::sync_channel(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let mut reader = DownloadReader {
            receiver,
            current: io::Cursor::new(Vec::new()),
            cancelled,
            ended: false,
        };
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(reader.read(&mut [0; 1]).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        thread.join().unwrap();
    }
    #[test]
    fn verified_copy_rejects_short_long_corrupt_and_cancelled_downloads() {
        let file = DownloadFile {
            role: "voice".into(),
            url: "https://example.org/voice".into(),
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        let cancel = AtomicBool::new(false);
        let mut output = Vec::new();
        copy_verified(&b"abc"[..], &mut output, &file, &cancel, |_| Ok(())).unwrap();
        assert_eq!(output, b"abc");
        for input in [b"ab".as_slice(), b"abcd", b"xyz"] {
            assert!(copy_verified(input, &mut Vec::new(), &file, &cancel, |_| Ok(())).is_err());
        }
        cancel.store(true, Ordering::Release);
        let mut output = Vec::new();
        assert!(copy_verified(&b"abc"[..], &mut output, &file, &cancel, |_| Ok(())).is_err());
        assert!(output.is_empty());
    }
}
