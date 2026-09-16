//! Local stdio management and one native owner per speech worker. No listener,
//! remote management command, native model loading or audio in the owner.
use crate::voice_validation::owned::platform;
use anyhow::{Context, Result};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::voice_library::installation::Activation;
use omnivox_tts::voice_library::local::{self, Host, Reply, Request, Startup};
use omnivox_tts::voice_library::ActivePointer;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub fn requested(args: &[String]) -> bool {
    args.first().is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "--voice-library-owner"
                | "--voice-library-service"
                | "--voice-library-local-version"
                | "--voice-library-acquire"
        )
    })
}
pub fn run(args: &[String]) -> Result<()> {
    anyhow::ensure!(
        args.len() == 1,
        "local provider takes no path arguments; use native environment settings"
    );
    if args[0] == "--voice-library-local-version" {
        println!("OMNIVOX-LOCAL 1");
        return Ok(());
    }
    let host = Host::open_default()?;
    if args[0] == "--voice-library-acquire" {
        return crate::voice_acquisition::run(host);
    }
    if args[0] == "--voice-library-service" {
        service(host)
    } else {
        owner(host)
    }
}

pub(crate) fn line(input: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            anyhow::ensure!(bytes.is_empty(), "truncated local input");
            return Ok(None);
        }
        let count = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |i| i + 1);
        anyhow::ensure!(
            bytes.len() + count <= local::MAX_LINE,
            "local input exceeds bound"
        );
        bytes.extend_from_slice(&buffer[..count]);
        input.consume(count);
        if bytes.last() == Some(&b'\n') {
            return Ok(Some(bytes));
        }
    }
}
fn reply(output: &Arc<Mutex<io::Stdout>>, id: u64, result: Result<Reply>) -> Result<()> {
    let response = match result {
        Ok(reply) => reply,
        Err(error) => Reply::Error {
            message: format!("{error:#}"),
        },
    };
    let line = response.line(id)?;
    let mut output = output
        .lock()
        .map_err(|_| anyhow::anyhow!("local output lock failed"))?;
    output.write_all(line.as_bytes())?;
    output.flush()?;
    Ok(())
}
fn service(host: Host) -> Result<()> {
    let output = Arc::new(Mutex::new(io::stdout()));
    let mut input = BufReader::new(io::stdin());
    let mut activation: Option<Activation> = None;
    while let Some(line) = line(&mut input)? {
        let request = Request::parse(&line)?;
        let result = (|| -> Result<Reply> {
            match request.command.as_str() {
                "host" => Ok(host.reply()),
                "catalogue" if activation.is_none() => Ok(Reply::Catalogue {
                    catalogue: omnivox_tts::voice_library::catalogue::Catalogue::parse(
                        request.plan_json.as_bytes(),
                    )?
                    .document()
                    .clone(),
                }),
                "acquisition-status" if activation.is_none() => {
                    let events = omnivox_tts::voice_library::acquisition::inspect(
                        &host,
                        &request.operation,
                    )?;
                    Ok(Reply::AcquisitionStatus {
                        events: events.into_iter().rev().take(64).collect(),
                    })
                }
                "inspect" if activation.is_none() => {
                    let profile = host.profile()?;
                    Ok(Reply::Library {
                        index: profile.index().document().clone(),
                        sha256: profile.index_sha256(),
                        active: profile
                            .active_json()?
                            .map(|json| ActivePointer::parse(json.as_bytes()))
                            .transpose()?,
                    })
                }
                "enable" if activation.is_none() => {
                    let mut profile = host.profile()?;
                    profile.set_enabled(
                        &PhysicalVoiceId::new(request.engine, request.voice),
                        request.enabled,
                        &local::new_uuid()?,
                        &request.expected_sha256,
                    )?;
                    Ok(Reply::State {
                        state: "pending".into(),
                    })
                }
                "include-flite-slt" if activation.is_none() => {
                    host.profile()?
                        .include_flite_slt(&local::new_uuid()?, &request.expected_sha256)?;
                    Ok(Reply::State {
                        state: "pending".into(),
                    })
                }
                "stage" if activation.is_none() => {
                    let profile = host.profile()?;
                    let candidate = profile.stage_activation(
                        &request.generation,
                        request.piper,
                        request.flite,
                        request.mbrola,
                        &request.expected_sha256,
                    )?;
                    Ok(Reply::Candidate {
                        path: profile
                            .generation_path(&request.generation)
                            .to_string_lossy()
                            .into(),
                        candidate,
                    })
                }
                "import" if activation.is_none() => {
                    let mut profile = host.profile()?;
                    profile.import_validated(
                        &request.operation,
                        &request.package,
                        &request.revision,
                        &local::new_uuid()?,
                        &request.expected_sha256,
                    )?;
                    Ok(Reply::State {
                        state: "installed-disabled".into(),
                    })
                }
                "snapshot" if activation.is_none() => {
                    let profile = host.profile()?;
                    let candidate = profile.activation_candidate(&request.generation)?;
                    let path = profile.generation_path(&request.generation);
                    drop(profile);
                    // Capture this launcher's current native inputs. The old
                    // worker's separately retained snapshot is only for rollback.
                    let mut startup = Startup::capture(&host)?;
                    startup.candidate(&path)?;
                    anyhow::ensure!(
                        startup.configuration.as_ref() == Some(&candidate.configuration),
                        "snapshot candidate changed"
                    );
                    let (path, digest) = startup.save(&host, &local::new_uuid()?)?;
                    Ok(Reply::Snapshot {
                        startup: path.to_string_lossy().into(),
                        startup_sha256: digest,
                        configuration: candidate.configuration,
                    })
                }
                "begin" if activation.is_none() => {
                    let transaction = host.profile()?.begin_activation(
                        &request.operation,
                        &request.generation,
                        &request.plan_json,
                    )?;
                    let response = Reply::Candidate {
                        path: transaction.generation_path().to_string_lossy().into(),
                        candidate: transaction.candidate().clone(),
                    };
                    activation = Some(transaction);
                    Ok(response)
                }
                "activating" => {
                    activation
                        .as_mut()
                        .context("no Apply lease")?
                        .activating()?;
                    Ok(Reply::State {
                        state: "activating".into(),
                    })
                }
                "rolling-back" => {
                    activation
                        .as_mut()
                        .context("no Apply lease")?
                        .rolling_back()?;
                    Ok(Reply::State {
                        state: "rolling-back".into(),
                    })
                }
                "commit" => {
                    let transaction = activation.as_mut().context("no Apply lease")?;
                    transaction.commit(&request.proofs_json)?;
                    Ok(Reply::Committed {
                        configuration: transaction.candidate().configuration.clone(),
                    })
                }
                "finish" => {
                    activation
                        .as_mut()
                        .context("no Apply lease")?
                        .finish(&request.state)?;
                    activation.take();
                    Ok(Reply::State {
                        state: request.state,
                    })
                }
                _ => anyhow::bail!("unsupported local command in current transaction"),
            }
        })();
        reply(&output, request.request_id, result)?;
    }
    // EOF leaves an unfinished journal for explicit inspection. It never
    // publishes a pointer, restarts speech or releases unresolved records.
    Ok(())
}

struct Worker {
    child: Child,
    tree: platform::Tree,
    readers: Vec<JoinHandle<Result<()>>>,
    cleaned: bool,
}
impl Worker {
    fn spawn(
        startup: &Startup,
        output: &Arc<Mutex<io::Stdout>>,
        slot: &mut Option<Self>,
    ) -> Result<()> {
        startup.verify()?;
        let tree = platform::Tree::for_speech()?;
        let mut command = Command::new(&startup.executable.path);
        command.args(&startup.arguments);
        command
            .env_clear()
            .envs(&startup.environment)
            // Local ownership needs the START barrier, not the remote
            // broker's restriction to bundled icon identifiers.
            .env_remove("OMNIVOX_REMOTE_WORKER")
            .env("OMNIVOX_OWNED_WORKER", "1")
            .current_dir(&startup.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        platform::configure_speech(&mut command);
        let child = command
            .spawn()
            .context("could not start owned speech worker")?;
        *slot = Some(Self {
            child,
            tree,
            readers: Vec::new(),
            cleaned: false,
        });
        // Retain the child before assignment, reader creation or START can
        // fail. The owner can then acknowledge cleanup of a partial attempt.
        let worker = slot.as_mut().unwrap();
        worker.tree.assign(&worker.child)?;
        let mut input = BufReader::new(worker.child.stdout.take().context("missing owned stdout")?);
        let output = Arc::clone(output);
        worker.readers.push(
            thread::Builder::new()
                .name("owned-speech-output".into())
                .spawn(move || {
                    while let Some(line) = line(&mut input)? {
                        anyhow::ensure!(
                            !line.starts_with(local::PREFIX.as_bytes()),
                            "worker emitted reserved owner response"
                        );
                        let mut output = output
                            .lock()
                            .map_err(|_| anyhow::anyhow!("speech output lock failed"))?;
                        output.write_all(&line)?;
                        output.flush()?;
                    }
                    Ok(())
                })?,
        );
        worker
            .child
            .stdin
            .as_mut()
            .context("missing owned stdin")?
            .write_all(b"START\n")?;
        Ok(())
    }
    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        self.child.stdin.take();
        self.tree.terminate()?;
        let _ = self.child.kill(); // Covers failed assignment before START.
        while self.child.try_wait()?.is_none() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "speech worker retirement unconfirmed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        while !self.tree.empty()? || self.readers.iter().any(|reader| !reader.is_finished()) {
            anyhow::ensure!(
                Instant::now() < deadline,
                "speech helper or pipe retirement unconfirmed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        for reader in self.readers.drain(..) {
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("speech reader panicked"))??;
        }
        self.cleaned = true;
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("Owned speech cleanup remains unconfirmed: {error:#}");
        }
    }
}
fn owner(host: Host) -> Result<()> {
    platform::initialize()?; // Dedicated owner; never an ordinary speech worker.
    let worker_id = local::new_uuid()?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let mut worker: Option<Worker> = None;
    let mut path = std::path::PathBuf::new();
    let mut digest = String::new();
    let mut configuration = None;
    let start = (|| -> Result<()> {
        let mut startup = if let Some(path) = std::env::var_os("OMNIVOX_OWNED_STARTUP") {
            Startup::read(
                Path::new(&path),
                &std::env::var("OMNIVOX_OWNED_STARTUP_SHA256")
                    .context("missing native startup digest")?,
            )?
        } else {
            Startup::capture(&host)?
        };
        if let Some(path) = std::env::var_os("OMNIVOX_OWNED_LIBRARY") {
            startup.candidate(Path::new(&path))?;
        }
        (path, digest) = startup.save(&host, &worker_id)?;
        configuration = startup.configuration.clone();
        Worker::spawn(&startup, &output, &mut worker)
    })();
    let mut startup_error = start.err().map(|error| format!("{error:#}"));
    if startup_error.is_some() {
        if let Err(error) = cleanup(&mut worker) {
            startup_error = Some(format!(
                "{}; cleanup unconfirmed: {error:#}",
                startup_error.unwrap()
            ));
        }
    }
    if let Some(error) = &startup_error {
        eprintln!("Owned speech startup failed: {error}");
    }
    let (sender, receiver) = mpsc::sync_channel(32);
    thread::Builder::new()
        .name("owned-speech-input".into())
        .spawn(move || {
            let mut input = BufReader::new(io::stdin());
            loop {
                let next = line(&mut input);
                let done = !matches!(next, Ok(Some(_)));
                if sender.send(next).is_err() || done {
                    break;
                }
            }
        })?;
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(Some(line))) if line.starts_with(local::PREFIX.as_bytes()) => {
                let request = Request::parse(&line[local::PREFIX.len()..])?;
                let mut retired = false;
                let result = match request.command.as_str() {
                    "describe" => Ok(Reply::Owner {
                        worker: worker_id.clone(),
                        startup: path.to_string_lossy().into(),
                        startup_sha256: digest.clone(),
                        configuration: configuration.clone(),
                        retired: worker.as_ref().is_none_or(|worker| worker.cleaned),
                        startup_error: startup_error.clone(),
                    }),
                    "retire" if request.worker == worker_id => match cleanup(&mut worker) {
                        Ok(()) => {
                            retired = true;
                            Ok(Reply::Retired {
                                worker: worker_id.clone(),
                            })
                        }
                        Err(error) => Err(error),
                    },
                    _ => Err(anyhow::anyhow!(
                        "unsupported request or wrong native worker identity"
                    )),
                };
                reply(&output, request.request_id, result)?;
                if retired {
                    return Ok(());
                }
            }
            Ok(Ok(Some(_))) if worker.as_ref().is_none_or(|worker| worker.cleaned) => (),
            Ok(Ok(Some(line))) => worker
                .as_mut()
                .unwrap()
                .child
                .stdin
                .as_mut()
                .context("owned speech stdin closed")?
                .write_all(&line)?,
            Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return cleanup(&mut worker)
            }
            Ok(Err(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Do not reap the group leader until the first termination
                // signal. Reader EOF is enough to begin owned cleanup.
                if worker.as_ref().is_some_and(|worker| {
                    !worker.cleaned && worker.readers.iter().all(|reader| reader.is_finished())
                }) {
                    cleanup(&mut worker)?;
                    reply(
                        &output,
                        0,
                        Ok(Reply::Retired {
                            worker: worker_id.clone(),
                        }),
                    )?;
                }
            }
        }
    }
}

fn cleanup(worker: &mut Option<Worker>) -> Result<()> {
    worker.as_mut().map_or(Ok(()), Worker::cleanup)
}
