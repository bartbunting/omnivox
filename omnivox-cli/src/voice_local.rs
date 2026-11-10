//! Local stdio management and one forwarding owner per speech worker. The
//! opt-in native guard carries only lifecycle control on independent local IPC.
//! No remote management command, native model loading or audio in the owner.
use crate::voice_validation::owned::platform;
use anyhow::{Context, Result};
use omnivox_tts::contracts::PhysicalVoiceId;
use omnivox_tts::voice_library::installation::Activation;
use omnivox_tts::voice_library::local::{self, Host, Reply, Request, Startup};
use omnivox_tts::voice_library::ActivePointer;
use std::io::{self, BufRead, BufReader, Read, Write};
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
                | "--emergency-owner"
                | "--emergency-version"
        )
    })
}
pub fn run(args: &[String]) -> Result<()> {
    if args == ["--emergency-version"] {
        println!("OMNIVOX-EMERGENCY 1");
        return Ok(());
    }
    if args[0] == "--emergency-owner" {
        let mut output = None;
        let mut data = None;
        let mut prepared = false;
        let mut guarded = false;
        let mut options = args[1..].iter();
        while let Some(flag) = options.next() {
            match flag.as_str() {
                "--prepare" if !prepared => prepared = true,
                "--guarded" if !guarded => guarded = true,
                "--audio-output" if output.is_none() => {
                    output = Some(options.next().context("missing emergency output")?.as_str());
                }
                "--espeak-data" if data.is_none() => {
                    data = Some(Path::new(
                        options.next().context("missing emergency data path")?,
                    ));
                }
                _ => anyhow::bail!("invalid or repeated emergency startup option"),
            }
        }
        check_guard_platform(guarded)?;
        return owner(
            None,
            Some((output.unwrap_or("device"), data)),
            prepared || guarded,
            guarded,
        );
    }
    let guarded = args == ["--voice-library-owner", "--guarded"];
    check_guard_platform(guarded)?;
    anyhow::ensure!(
        args.len() == 1 || guarded,
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
        owner(Some(host), None, false, guarded)
    }
}

fn check_guard_platform(guarded: bool) -> Result<()> {
    // Windows additionally checks the supported engine profile before
    // creating a guard. Ordinary startup keeps its existing engine coverage.
    anyhow::ensure!(
        !guarded || cfg!(any(target_os = "linux", windows)),
        "guarded owners are currently implemented only on Linux and Windows"
    );
    Ok(())
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
                "retirement-status" if activation.is_none() => {
                    let retired = omnivox_tts::voice_library::retention::retirement_confirmed(
                        &host,
                        &request.worker,
                        Path::new(&request.startup),
                        &request.startup_sha256,
                    )?;
                    Ok(Reply::RetirementStatus {
                        worker: request.worker,
                        startup_sha256: request.startup_sha256,
                        retired,
                    })
                }
                "engine-settings-review" | "engine-settings-save" | "engine-settings-add"
                    if activation.is_none() =>
                {
                    use omnivox_tts::engine_configuration::{settings_editor, Platform};
                    let root =
                        Platform::native().configuration_root(None, |key| std::env::var_os(key))?;
                    let review = match request.command.as_str() {
                        "engine-settings-save" => settings_editor::save(
                            &root,
                            &request.expected_sha256,
                            request.settings_json.as_bytes(),
                        )?,
                        "engine-settings-add" => settings_editor::add_helper(
                            &root,
                            &request.expected_sha256,
                            request.manifest_json.as_bytes(),
                        )?,
                        _ => settings_editor::inspect(&root)?,
                    };
                    Ok(Reply::EngineSettings { review })
                }
                "punctuation-review" | "punctuation-save" if activation.is_none() => {
                    use omnivox_tts::engine_configuration::{punctuation_editor, Platform};
                    let root =
                        Platform::native().configuration_root(None, |key| std::env::var_os(key))?;
                    let review = if request.command == "punctuation-save" {
                        punctuation_editor::save_profiles(
                            &root,
                            &request.expected_sha256,
                            request.punctuation_json.as_bytes(),
                            (!request.profiles_json.is_empty())
                                .then_some(request.profiles_json.as_bytes()),
                        )?
                    } else {
                        punctuation_editor::inspect(&root)?
                    };
                    Ok(Reply::PunctuationConfiguration { review })
                }
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
                "uninstall-preview" if activation.is_none() => Ok(Reply::RemovalReview {
                    review: host.profile()?.prepare_removal(
                        &PhysicalVoiceId::new(request.engine, request.voice),
                        &request.expected_sha256,
                    )?,
                }),
                "uninstall-pending" if activation.is_none() => Ok(Reply::Removals {
                    reviews: host.profile()?.pending_removals()?,
                }),
                "uninstall" if activation.is_none() => Ok(Reply::Removal {
                    result: host
                        .profile()?
                        .execute_removal(&request.operation, &request.expected_sha256)?,
                }),
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
                        request.rhvoice,
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
                    let mut startup = if request.startup.is_empty() {
                        anyhow::ensure!(
                            request.startup_sha256.is_empty(),
                            "startup digest requires a startup reference"
                        );
                        let mut startup = Startup::capture(&host)?;
                        startup.candidate(&path)?;
                        startup
                    } else {
                        shared_startup(Path::new(&request.startup), &request.startup_sha256)?
                    };
                    anyhow::ensure!(
                        startup.configuration.as_ref() == Some(&candidate.configuration),
                        "snapshot candidate changed"
                    );
                    prepare_engines(&mut startup)?;
                    let (path, digest) = startup.save_prepared(&host, &local::new_uuid()?)?;
                    Ok(Reply::Snapshot {
                        startup: path.to_string_lossy().into(),
                        startup_sha256: digest,
                        configuration: candidate.configuration,
                        activation_id: startup.engines.as_ref().unwrap().activation_id().into(),
                    })
                }
                "engine-snapshot" if activation.is_none() => {
                    let mut startup = Startup::capture(&host)?;
                    prepare_engines(&mut startup)?;
                    let (path, digest) = startup.save_prepared(&host, &local::new_uuid()?)?;
                    Ok(Reply::PreparedStartup {
                        startup: path.to_string_lossy().into(),
                        startup_sha256: digest,
                        configuration: startup.configuration.clone(),
                        activation_id: startup.engines.as_ref().unwrap().activation_id().into(),
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

enum NativeOwner {
    Direct {
        child: Child,
        tree: platform::Tree,
    },
    #[cfg(any(target_os = "linux", windows))]
    Guarded(Box<crate::native_guard::Peer>),
}

struct Worker {
    native: NativeOwner,
    input: Option<Box<dyn Write + Send>>,
    readers: Vec<JoinHandle<Result<()>>>,
    cleaned: bool,
    snapshot: Option<(Host, std::path::PathBuf, String)>,
}
impl Worker {
    fn spawn(
        startup: &Startup,
        output: &Arc<Mutex<io::Stdout>>,
        slot: &mut Option<Self>,
    ) -> Result<()> {
        crate::emergency::timed("verify-executable-and-startup", || startup.verify())?;
        let tree = platform::Tree::for_speech()?;
        let mut command = Command::new(&startup.executable.path);
        command.args(&startup.arguments);
        startup.environment.apply(&mut command);
        command
            .env_remove("OMNIVOX_REMOTE_WORKER")
            .env("OMNIVOX_OWNED_WORKER", "1")
            .current_dir(&startup.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        platform::configure_speech(&mut command);
        let mut child = crate::emergency::timed("spawn-worker", || command.spawn())
            .context("could not start owned speech worker")?;
        let input = child
            .stdin
            .take()
            .map(|input| Box::new(input) as Box<dyn Write + Send>);
        *slot = Some(Self {
            native: NativeOwner::Direct { child, tree },
            input,
            readers: Vec::new(),
            cleaned: false,
            snapshot: None,
        });
        // Retain the child before assignment, reader creation or START can fail.
        let worker = slot.as_mut().unwrap();
        let source = match &mut worker.native {
            NativeOwner::Direct { child, tree } => {
                tree.assign(child)?;
                child.stdout.take().context("missing owned stdout")?
            }
            #[cfg(any(target_os = "linux", windows))]
            NativeOwner::Guarded(_) => unreachable!(),
        };
        worker.forward_output(source, output)?;
        worker.send_startup(startup)
    }

    fn prepare_guarded(
        startup: &Startup,
        worker_id: &str,
        output: &Arc<Mutex<io::Stdout>>,
        slot: &mut Option<Self>,
    ) -> Result<()> {
        #[cfg(any(target_os = "linux", windows))]
        {
            let (peer, input, source) = crate::native_guard::Peer::prepare(startup, worker_id)?;
            *slot = Some(Self {
                native: NativeOwner::Guarded(Box::new(peer)),
                input: Some(Box::new(input)),
                readers: Vec::new(),
                cleaned: false,
                snapshot: None,
            });
            slot.as_mut().unwrap().forward_output(source, output)
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = (startup, worker_id, output, slot);
            anyhow::bail!("guarded native ownership is not implemented on this platform")
        }
    }

    fn guard_endpoint(&self) -> Option<local::GuardEndpoint> {
        match &self.native {
            #[cfg(any(target_os = "linux", windows))]
            NativeOwner::Guarded(peer) => Some(peer.endpoint()),
            _ => None,
        }
    }

    fn start_guarded(&mut self, startup: &Startup) -> Result<()> {
        #[cfg(any(target_os = "linux", windows))]
        if let NativeOwner::Guarded(peer) = &mut self.native {
            crate::emergency::timed("guard-worker-start", || peer.start())?;
            return self.send_startup(startup);
        }
        let _ = startup;
        anyhow::bail!("worker has no independent native guard")
    }

    fn forward_output(
        &mut self,
        source: impl Read + Send + 'static,
        output: &Arc<Mutex<io::Stdout>>,
    ) -> Result<()> {
        let mut input = BufReader::new(source);
        let output = Arc::clone(output);
        self.readers.push(
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
        Ok(())
    }

    fn send_startup(&mut self, startup: &Startup) -> Result<()> {
        let snapshot = startup
            .engines
            .clone()
            .context("owner lacks frozen engine startup")?;
        let mut stdin = self.input.take().context("missing owned stdin")?;
        let (sender, receiver) = mpsc::sync_channel(1);
        // Keep every startup writer and its completion lease until joined.
        self.readers.push(
            thread::Builder::new()
                .name("owned-speech-startup".into())
                .spawn(move || {
                    let result =
                        crate::worker_startup::write(&mut stdin, &snapshot).map(|()| stdin);
                    let _ = sender.send(result);
                    Ok(())
                })?,
        );
        self.input = Some(crate::emergency::timed("send-owned-startup", || {
            receiver
                .recv_timeout(Duration::from_secs(10))
                .context("owned worker startup transmission did not complete")?
        })?);
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            self.record_retirement()?;
            return Ok(());
        }
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        match &mut self.native {
            NativeOwner::Direct { child, tree } => {
                tree.terminate()?;
                let _ = child.kill(); // Covers failed assignment before START.
                while child.try_wait()?.is_none() || !tree.empty()? {
                    anyhow::ensure!(
                        Instant::now() < deadline,
                        "speech tree retirement unconfirmed"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            }
            #[cfg(any(target_os = "linux", windows))]
            NativeOwner::Guarded(peer) => peer.retire_tree()?,
        }
        while self.readers.iter().any(|reader| !reader.is_finished()) {
            anyhow::ensure!(
                Instant::now() < deadline,
                "speech pipe retirement unconfirmed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        for reader in self.readers.drain(..) {
            reader
                .join()
                .map_err(|_| anyhow::anyhow!("speech reader panicked"))??;
        }
        #[cfg(any(target_os = "linux", windows))]
        if let NativeOwner::Guarded(peer) = &mut self.native {
            peer.finish_io()?;
        }
        self.cleaned = true;
        self.record_retirement()?;
        Ok(())
    }
    fn record_retirement(&mut self) -> Result<()> {
        if let Some((host, path, hash)) = &self.snapshot {
            omnivox_tts::voice_library::retention::retired(host, path, hash)?;
            self.snapshot = None;
        }
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
fn owner(
    host: Option<Host>,
    emergency_output: Option<(&str, Option<&Path>)>,
    deferred: bool,
    guarded: bool,
) -> Result<()> {
    platform::initialize()?; // Dedicated owner; never an ordinary speech worker.
    let worker_id = local::new_uuid()?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let mut worker: Option<Worker> = None;
    let mut path = std::path::PathBuf::new();
    let mut digest = String::new();
    let mut configuration = None;
    let mut activation_id = None;
    let mut prepared = None;
    let start = (|| -> Result<()> {
        if let Some((output_backend, data)) = emergency_output {
            let startup = crate::emergency::timed("prepare-emergency-startup", || {
                crate::emergency::startup(output_backend, data)
            })?;
            activation_id = startup
                .engines
                .as_ref()
                .map(|snapshot| snapshot.activation_id().to_owned());
            if deferred {
                // No native child or output connection exists during preparation.
                // Check the selected data before the client retires working speech.
                let parent =
                    data.unwrap_or_else(|| Path::new(&startup.executable.path).parent().unwrap());
                anyhow::ensure!(
                    parent.join("espeak-ng-data/phontab").is_file(),
                    "emergency speech requires its selected bundled eSpeak data"
                );
                if guarded {
                    Worker::prepare_guarded(&startup, &worker_id, &output, &mut worker)?;
                }
                prepared = Some(startup);
                return Ok(());
            }
            return Worker::spawn(&startup, &output, &mut worker);
        }
        let host = host
            .as_ref()
            .context("ordinary owner lacks management host")?;
        anyhow::ensure!(
            std::env::var_os("OMNIVOX_OWNED_ENGINE_STARTUP").is_none()
                || (std::env::var_os("OMNIVOX_OWNED_STARTUP").is_none()
                    && std::env::var_os("OMNIVOX_OWNED_LIBRARY").is_none()),
            "shared engine startup conflicts with another retained startup or generation"
        );
        let mut startup = if let Some(path) = std::env::var_os("OMNIVOX_OWNED_STARTUP") {
            let retained = Startup::read(
                Path::new(&path),
                &std::env::var("OMNIVOX_OWNED_STARTUP_SHA256")
                    .context("missing native startup digest")?,
            )?;
            anyhow::ensure!(
                retained.engines.is_some(),
                "retained startup predates engine snapshots; prepare a fresh activation"
            );
            retained
        } else if let Some(path) = std::env::var_os("OMNIVOX_OWNED_ENGINE_STARTUP") {
            shared_startup(
                Path::new(&path),
                &std::env::var("OMNIVOX_OWNED_ENGINE_STARTUP_SHA256")
                    .context("missing shared engine startup digest")?,
            )?
        } else {
            crate::emergency::timed("capture-startup", || Startup::capture(host))?
        };
        if let Some(path) = std::env::var_os("OMNIVOX_OWNED_LIBRARY") {
            startup.candidate(Path::new(&path))?;
        }
        crate::emergency::timed("prepare-engine-snapshot", || prepare_engines(&mut startup))?;
        (path, digest) =
            crate::emergency::timed("save-startup", || startup.save(host, &worker_id))?;
        configuration = startup.configuration.clone();
        activation_id = startup
            .engines
            .as_ref()
            .map(|snapshot| snapshot.activation_id().to_owned());
        let result = if guarded {
            let result = Worker::prepare_guarded(&startup, &worker_id, &output, &mut worker);
            if result.is_ok() {
                prepared = Some(startup);
            }
            result
        } else {
            Worker::spawn(&startup, &output, &mut worker)
        };
        if let Some(worker) = &mut worker {
            worker.snapshot = Some((host.clone(), path.clone(), digest.clone()));
        } else {
            // No worker was created; this snapshot cannot own native assets.
            omnivox_tts::voice_library::retention::retired(host, &path, &digest)?;
        }
        result
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
                        retired: prepared.is_none()
                            && worker.as_ref().is_none_or(|worker| worker.cleaned),
                        startup_error: startup_error.clone(),
                        activation_id: activation_id.clone(),
                        native_retirement_version: 1,
                        emergency: emergency_output.is_some(),
                        deferred_start_version: 1,
                        prepared: prepared.is_some(),
                        guard: worker.as_ref().and_then(Worker::guard_endpoint),
                    }),
                    "start-emergency" | "start-guarded"
                        if request.worker == worker_id
                            && ((request.command == "start-emergency" && deferred)
                                || (request.command == "start-guarded" && guarded)) =>
                    {
                        if let Some(startup) = prepared.take() {
                            // Consume the single attempt before native creation. Retrying
                            // a lost reply can observe it but never launch another worker.
                            let start = if guarded {
                                worker
                                    .as_mut()
                                    .context("prepared guard missing")?
                                    .start_guarded(&startup)
                            } else {
                                Worker::spawn(&startup, &output, &mut worker)
                            };
                            if let Err(error) = start {
                                startup_error = Some(format!("{error:#}"));
                                if let Err(cleanup_error) = cleanup(&mut worker) {
                                    eprintln!(
                                        "Deferred startup cleanup unconfirmed: {cleanup_error:#}"
                                    );
                                }
                            }
                        }
                        if let Some(error) = &startup_error {
                            Err(anyhow::anyhow!(
                                "deferred emergency startup failed: {error}"
                            ))
                        } else if worker.as_ref().is_some_and(|worker| !worker.cleaned) {
                            Ok(Reply::Started {
                                worker: worker_id.clone(),
                            })
                        } else {
                            Err(anyhow::anyhow!("emergency start was cancelled or retired"))
                        }
                    }
                    "retire-native" if request.worker == worker_id => {
                        prepared.take();
                        let result = cleanup(&mut worker);
                        if worker.as_ref().is_none_or(|worker| worker.cleaned) {
                            // Native proof survives a failed management write. Keep this
                            // live authority queryable until its client closes the pipe.
                            Ok(Reply::NativeRetired {
                                worker: worker_id.clone(),
                                retirement_recorded: emergency_output.is_none()
                                    && worker
                                        .as_ref()
                                        .is_some_and(|worker| worker.snapshot.is_none())
                                    && !path.as_os_str().is_empty(),
                            })
                        } else {
                            result.map(|()| unreachable!("cleanup succeeded without native proof"))
                        }
                    }
                    "retire" if request.worker == worker_id => {
                        prepared.take();
                        match cleanup(&mut worker) {
                            Ok(()) => {
                                retired = true;
                                Ok(Reply::Retired {
                                    worker: worker_id.clone(),
                                })
                            }
                            Err(error) => Err(error),
                        }
                    }
                    _ => Err(anyhow::anyhow!(
                        "unsupported request or wrong native worker identity"
                    )),
                };
                reply(&output, request.request_id, result)?;
                if retired {
                    return Ok(());
                }
            }
            Ok(Ok(Some(_)))
                if prepared.is_some() || worker.as_ref().is_none_or(|worker| worker.cleaned) => {}
            Ok(Ok(Some(line))) => worker
                .as_mut()
                .unwrap()
                .input
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
                    if let Err(error) = cleanup(&mut worker) {
                        if !worker.as_ref().is_none_or(|worker| worker.cleaned) {
                            return Err(error);
                        }
                        // Keep the authoritative native observation available even
                        // when disk failure prevents a durable retirement receipt.
                        eprintln!(
                            "Native speech retired; management receipt unavailable: {error:#}"
                        );
                        reply(
                            &output,
                            0,
                            Ok(Reply::NativeRetired {
                                worker: worker_id.clone(),
                                retirement_recorded: false,
                            }),
                        )?;
                        continue;
                    }
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

fn prepare_engines(startup: &mut Startup) -> Result<()> {
    if startup.engines.is_none() {
        startup.engines = Some(
            crate::engine::EngineStartup::capture(
                "",
                None,
                None,
                None,
                startup.environment.clone(),
                Path::new(&startup.executable.path),
            )?
            .snapshot,
        );
    }
    startup.verify()?;
    Ok(())
}

fn shared_startup(path: &Path, digest: &str) -> Result<Startup> {
    anyhow::ensure!(
        path.is_absolute(),
        "shared startup reference must be absolute"
    );
    let startup = Startup::read(path, digest)?;
    anyhow::ensure!(
        startup.engines.is_some(),
        "shared startup lacks a complete engine snapshot"
    );
    Ok(startup.with_lane_audio(&omnivox_tts::engine_configuration::LaunchEnvironment::capture()))
}

fn cleanup(worker: &mut Option<Worker>) -> Result<()> {
    worker.as_mut().map_or(Ok(()), Worker::cleanup)
}
