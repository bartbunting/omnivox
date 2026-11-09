//! Local owned-worker preference preparation, independent of speech and native lifetime.
use crate::server::Input;
use anyhow::Result;
use omnivox_core::{punctuation::PunctuationTables, state::TtsState};
use omnivox_tts::control::{
    decode_request, ControlRequest, ControlResponse, ControlResponseEnvelope,
    PunctuationProfileDescriptor, CONTROL_PROTOCOL_VERSION,
};
use omnivox_tts::engine_configuration::LaunchSnapshot;
use omnivox_tts::host_preferences::{Operation, Outcome, ReloadPunctuation, ReloadResult, Status};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};

static AVAILABLE: AtomicBool = AtomicBool::new(false);
type Reporter = Arc<dyn Fn(&ControlResponseEnvelope) + Send + Sync>;
type Prepare = Box<dyn Fn() -> std::result::Result<Arc<PunctuationTables>, &'static str> + Send>;

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Acquire)
}

fn digest(value: &impl serde::Serialize) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("policy encoding"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn catalogue(tables: &PunctuationTables) -> Vec<PunctuationProfileDescriptor> {
    tables
        .profiles
        .iter()
        .map(|(id, profile)| PunctuationProfileDescriptor {
            id: id.clone(),
            fallback: profile.fallback,
            sha256: digest(profile),
        })
        .collect()
}
fn native_identity(snapshot: &LaunchSnapshot) -> Value {
    let mut value = serde_json::to_value(snapshot).expect("snapshot encoding");
    for key in ["punctuation", "activation_id", "root"] {
        value.as_object_mut().unwrap().remove(key);
    }
    value
}
struct Prepared {
    tables: Arc<PunctuationTables>,
    profiles: Vec<PunctuationProfileDescriptor>,
    digest: String,
    installed: mpsc::SyncSender<bool>,
}
struct Shared {
    status: Status,
    profiles: Vec<PunctuationProfileDescriptor>,
    pending: Option<Prepared>,
}
pub struct Runtime {
    shared: Arc<Mutex<Shared>>,
    tasks: Option<mpsc::SyncSender<()>>,
    reports: mpsc::SyncSender<ControlResponseEnvelope>,
    closed: Arc<AtomicBool>,
    advertised: bool,
}

impl Runtime {
    pub fn start(sender: mpsc::SyncSender<Input>, state: &TtsState) -> Result<Option<Self>> {
        if std::env::var_os("OMNIVOX_REMOTE_WORKER").is_some() {
            return Ok(None);
        }
        let Some(snapshot) = crate::worker_startup::snapshot() else {
            return Ok(None);
        };
        let snapshot = snapshot.clone();
        let native = native_identity(&snapshot);
        let executable = std::env::current_exe()?.canonicalize()?;
        let startup = snapshot.activation_id().to_owned();
        let configuration_file =
            omnivox_tts::engine_configuration::punctuation_editor::destination(
                &crate::engine::EngineStartup::preferences_root(&snapshot)?,
            )?
            .to_string_lossy()
            .into_owned();
        let prepare = Box::new(move || {
            let candidate =
                crate::engine::EngineStartup::preferences_candidate(&snapshot, &executable)
                    .map_err(|_| "configuration is invalid or unreadable")?;
            if native_identity(&candidate.snapshot) != native {
                return Err("changes outside punctuation require restart");
            }
            Ok(Arc::new(
                candidate.snapshot.resolved().speech.punctuation.clone(),
            ))
        });
        let mut runtime = Self::new(
            sender,
            state,
            startup,
            prepare,
            Arc::new(crate::server::write_control_response),
        )?;
        runtime.shared.lock().unwrap().status.configuration_file = configuration_file;
        runtime.advertised = true;
        AVAILABLE.store(true, Ordering::Release);
        Ok(Some(runtime))
    }

    fn new(
        sender: mpsc::SyncSender<Input>,
        state: &TtsState,
        startup: String,
        prepare: Prepare,
        report: Reporter,
    ) -> Result<Self> {
        let hash = digest(state.punctuation_tables.as_ref());
        let shared = Arc::new(Mutex::new(Shared {
            status: Status {
                incarnation: omnivox_tts::voice_library::local::new_uuid()?,
                configuration_file: String::new(),
                startup_activation_id: startup,
                revision: 0,
                punctuation_sha256: hash.clone(),
                startup_punctuation_sha256: hash,
                last_operation: None,
            },
            profiles: catalogue(&state.punctuation_tables),
            pending: None,
        }));
        let closed = Arc::new(AtomicBool::new(false));
        let (reports, receiver) = mpsc::sync_channel(16);
        let stopped = Arc::clone(&closed);
        std::thread::Builder::new()
            .name("omnivox-preference-replies".into())
            .spawn(move || {
                while let Ok(response) = receiver.recv() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    report(&response);
                }
            })?;
        let (tasks, receiver) = mpsc::sync_channel(1);
        let worker = Arc::clone(&shared);
        let stopped = Arc::clone(&closed);
        // Keep the current policy alive off admission so swapping the last request
        // reference never destroys a large table on the command thread.
        let mut retained = Arc::clone(&state.punctuation_tables);
        std::thread::Builder::new()
            .name("omnivox-preference-prepare".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    match prepare() {
                        Ok(tables) => {
                            let hash = digest(tables.as_ref());
                            let profiles = catalogue(&tables);
                            let (installed, receipt) = mpsc::sync_channel(1);
                            {
                                let mut shared = worker.lock().unwrap();
                                if stopped.load(Ordering::Acquire) {
                                    break;
                                }
                                shared.status.last_operation.as_mut().unwrap().outcome =
                                    Outcome::Pending;
                                shared.pending = Some(Prepared {
                                    tables: Arc::clone(&tables),
                                    profiles,
                                    digest: hash,
                                    installed,
                                });
                            }
                            let _ = sender.try_send(Input::PreferenceWake);
                            if receipt.recv() == Ok(true) {
                                retained = tables;
                            }
                        }
                        Err(reason) => {
                            let mut shared = worker.lock().unwrap();
                            let operation = shared.status.last_operation.as_mut().unwrap();
                            operation.outcome = Outcome::Rejected;
                            operation.reason = Some(reason.into());
                        }
                    }
                }
                drop(retained);
            })?;
        Ok(Self {
            shared,
            tasks: Some(tasks),
            reports,
            closed,
            advertised: false,
        })
    }

    fn status(&self) -> Status {
        self.shared.lock().unwrap().status.clone()
    }

    // All replies are bounded and off admission. A client that stops reading can
    // lose a response and query later; it cannot block Stop in this handler.
    pub fn try_handle(&self, payload: &str) -> bool {
        let Ok(request) = decode_request(payload) else {
            return false;
        };
        if request.protocol_version != CONTROL_PROTOCOL_VERSION {
            return false;
        }
        let response = match request.request {
            ControlRequest::GetPunctuationProfilesV1 => ControlResponse::PunctuationProfilesV1 {
                profiles: self.shared.lock().unwrap().profiles.clone(),
            },
            ControlRequest::HostPreferencesStatusV1 => {
                ControlResponse::HostPreferencesStatusV1(self.status())
            }
            ControlRequest::ReloadPunctuationV1(operation) => {
                ControlResponse::ReloadPunctuationV1(self.begin(operation))
            }
            _ => return false,
        };
        let _ = self.reports.try_send(ControlResponseEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: Some(request.request_id),
            response,
        });
        true
    }

    fn begin(&self, request: ReloadPunctuation) -> ReloadResult {
        let mut shared = self.shared.lock().unwrap();
        let status = &mut shared.status;
        let previous = status.last_operation.as_ref();
        let last_sequence = previous.map_or(0, |op| op.sequence);
        let outcome = if request.incarnation != status.incarnation || request.sequence == 0 {
            Outcome::Stale
        } else if let Some(old) = previous.filter(|old| old.sequence == request.sequence) {
            if old.expected_revision == request.expected_revision {
                old.outcome
            } else {
                Outcome::Stale
            }
        } else if request.sequence <= last_sequence || request.expected_revision != status.revision
        {
            Outcome::Stale
        } else if previous
            .is_some_and(|old| matches!(old.outcome, Outcome::Preparing | Outcome::Pending))
        {
            Outcome::Busy
        } else if last_sequence.checked_add(1) != Some(request.sequence) {
            Outcome::Stale
        } else {
            status.last_operation = Some(Operation {
                sequence: request.sequence,
                expected_revision: request.expected_revision,
                outcome: Outcome::Preparing,
                reason: None,
            });
            if self.tasks.as_ref().unwrap().try_send(()).is_err() {
                let operation = status.last_operation.as_mut().unwrap();
                operation.outcome = Outcome::Rejected;
                operation.reason = Some("preparation is unavailable".into());
                Outcome::Rejected
            } else {
                Outcome::Preparing
            }
        };
        ReloadResult {
            sequence: request.sequence,
            outcome,
            state: status.clone(),
        }
    }

    pub fn install(&self, state: &mut TtsState, clean: bool) {
        if !clean {
            return;
        }
        let mut shared = self.shared.lock().unwrap();
        let Some(prepared) = shared.pending.take() else {
            return;
        };
        let outcome = if shared.status.punctuation_sha256 == prepared.digest {
            Outcome::Unchanged
        } else if state
            .punctuation_profile
            .as_ref()
            .is_some_and(|id| !prepared.tables.profiles.contains_key(id))
        {
            Outcome::Rejected
        } else if let Some(revision) = shared.status.revision.checked_add(1) {
            state.punctuation_tables = prepared.tables;
            shared.status.revision = revision;
            shared.status.punctuation_sha256 = prepared.digest;
            shared.profiles = prepared.profiles;
            Outcome::Applied
        } else {
            Outcome::Rejected
        };
        let operation = shared.status.last_operation.as_mut().unwrap();
        operation.outcome = outcome;
        if outcome == Outcome::Rejected {
            operation.reason = Some("current profile is missing or revision is exhausted".into());
        }
        let _ = prepared.installed.try_send(outcome == Outcome::Applied);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if self.advertised {
            AVAILABLE.store(false, Ordering::Release);
        }
        self.shared.lock().unwrap().pending.take();
        self.tasks.take();
        // Never join file I/O or a blocked client writer on speech teardown.
        // These bounded threads own no native engine; native process exit/retirement
        // remains the existing owner's responsibility.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_core::{punctuation::PunctuationProfile, PunctuationLevel};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn tables(word: &str) -> Arc<PunctuationTables> {
        let mut tables = PunctuationTables::default();
        tables.all.insert('!', Some(word.into()));
        tables.profiles.insert(
            "probe".into(),
            PunctuationProfile {
                fallback: PunctuationLevel::All,
                table: tables.all.clone(),
            },
        );
        Arc::new(tables)
    }
    fn request(runtime: &Runtime, sequence: u64) -> ReloadPunctuation {
        let status = runtime.status();
        ReloadPunctuation {
            incarnation: status.incarnation,
            sequence,
            expected_revision: status.revision,
        }
    }
    fn fixture(state: &TtsState, prepare: Prepare) -> (Runtime, mpsc::Receiver<Input>) {
        let (wake, receiver) = mpsc::sync_channel(32);
        (
            Runtime::new(wake, state, "startup".into(), prepare, Arc::new(|_| {})).unwrap(),
            receiver,
        )
    }
    fn install(runtime: &Runtime, receiver: &mpsc::Receiver<Input>, state: &mut TtsState) {
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Ok(Input::PreferenceWake)
        ));
        runtime.install(state, true);
    }

    #[test]
    fn coherent_requests_live_status_reset_and_retained_startup_are_distinct() {
        let mut state = TtsState::new();
        state.punctuation_tables = tables("old");
        state.punctuation_profile = Some("probe".into());
        state.speech_rate = 321.0;
        let admitted = state.clone();
        let (runtime, receiver) = fixture(&state, Box::new(|| Ok(tables("new"))));
        assert_eq!(
            runtime.begin(request(&runtime, 1)).outcome,
            Outcome::Preparing
        );
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Applied
        );
        assert_eq!(runtime.status().revision, 1);
        assert_ne!(
            runtime.status().punctuation_sha256,
            runtime.status().startup_punctuation_sha256
        );
        assert!(crate::text::prepare_speech_text("hello!", &admitted)
            .text
            .contains("old"));
        assert!(crate::text::prepare_speech_text("hello!", &state)
            .text
            .contains("new"));
        assert_eq!(state.speech_rate, 321.0);
        state.reset();
        state.punctuation_level = PunctuationLevel::All;
        assert!(crate::text::prepare_speech_text("hello!", &state)
            .text
            .contains("new"));
    }

    #[test]
    fn lost_replies_duplicates_and_aba_cannot_replay_old_operations() {
        let mut state = TtsState::new();
        state.punctuation_tables = tables("a");
        let captures = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&captures);
        let (runtime, receiver) = fixture(
            &state,
            Box::new(move || {
                Ok(tables(
                    if counter.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
                        "b"
                    } else {
                        "a"
                    },
                ))
            }),
        );
        let first = request(&runtime, 1);
        runtime.begin(first.clone());
        install(&runtime, &receiver, &mut state);
        assert_eq!(runtime.begin(first.clone()).outcome, Outcome::Applied);
        assert_eq!(captures.load(Ordering::SeqCst), 1);
        runtime.begin(request(&runtime, 2));
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.status().punctuation_sha256,
            runtime.status().startup_punctuation_sha256
        );
        assert_eq!(runtime.status().revision, 2);
        assert_eq!(runtime.begin(first).outcome, Outcome::Stale);
        let mut stale = request(&runtime, 3);
        stale.incarnation = "old-worker".into();
        assert_eq!(runtime.begin(stale).outcome, Outcome::Stale);
        assert_eq!(captures.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn pending_batch_and_selected_profile_rejection_keep_working_policy() {
        let mut state = TtsState::new();
        state.punctuation_tables = tables("old");
        state.punctuation_profile = Some("probe".into());
        let (runtime, receiver) = fixture(
            &state,
            Box::new(|| Ok(Arc::new(PunctuationTables::default()))),
        );
        runtime.begin(request(&runtime, 1));
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        runtime.install(&mut state, false);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Pending
        );
        assert_eq!(runtime.begin(request(&runtime, 2)).outcome, Outcome::Busy);
        runtime.install(&mut state, true);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Rejected
        );
        assert_eq!(runtime.status().revision, 0);
        assert!(crate::text::prepare_speech_text("hello!", &state)
            .text
            .contains("old"));
    }

    #[test]
    fn bounded_preparation_does_not_hold_speech_teardown() {
        let (entered, enter) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let (runtime, _) = fixture(
            &TtsState::new(),
            Box::new(move || {
                entered.send(()).unwrap();
                wait.recv().unwrap();
                Ok(tables("new"))
            }),
        );
        runtime.begin(request(&runtime, 1));
        enter.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..100 {
            assert_eq!(runtime.begin(request(&runtime, 2)).outcome, Outcome::Busy);
        }
        let (done, completion) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            drop(runtime);
            done.send(()).unwrap();
        });
        let result = completion.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        assert!(result.is_ok(), "file I/O held speech teardown");
    }

    #[test]
    fn blocked_management_writer_does_not_hold_admission_or_teardown() {
        let (entered, enter) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let wait = Mutex::new(wait);
        let (sender, _receiver) = mpsc::sync_channel(1);
        let runtime = Runtime::new(
            sender,
            &TtsState::new(),
            "startup".into(),
            Box::new(|| Ok(tables("new"))),
            Arc::new(move |_| {
                entered.try_send(()).unwrap();
                wait.lock().unwrap().recv().unwrap();
            }),
        )
        .unwrap();
        let payload =
            omnivox_tts::control::encode_request(&omnivox_tts::control::ControlRequestEnvelope {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                request_id: 1,
                request: ControlRequest::HostPreferencesStatusV1,
            })
            .unwrap();
        assert!(runtime.try_handle(&payload));
        enter.recv_timeout(Duration::from_secs(5)).unwrap();
        let (done, completion) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for _ in 0..1000 {
                assert!(runtime.try_handle(&payload));
            }
            drop(runtime);
            done.send(()).unwrap();
        });
        let result = completion.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        assert!(result.is_ok(), "blocked replies held admission or teardown");
    }

    #[test]
    fn maximum_valid_policy_installs_as_one_snapshot() {
        let table = (0x4e00..0x5000)
            .map(|c| (char::from_u32(c).unwrap(), Some("n".repeat(64))))
            .collect();
        let mut tables = PunctuationTables {
            all: table,
            ..PunctuationTables::default()
        };
        for n in 0..32 {
            tables.profiles.insert(
                format!("p{n}"),
                PunctuationProfile {
                    fallback: PunctuationLevel::All,
                    table: tables.all.clone(),
                },
            );
        }
        tables.validate().unwrap();
        let tables = Arc::new(tables);
        let captured = Arc::clone(&tables);
        let mut state = TtsState::new();
        let (runtime, receiver) = fixture(&state, Box::new(move || Ok(Arc::clone(&captured))));
        runtime.begin(request(&runtime, 1));
        install(&runtime, &receiver, &mut state);
        assert!(Arc::ptr_eq(&tables, &state.punctuation_tables));
        assert_eq!(runtime.status().punctuation_sha256, digest(tables.as_ref()));
    }
}
