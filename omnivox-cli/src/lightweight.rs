//! Local owned-worker preference preparation, independent of speech and native lifetime.
use crate::server::Input;
use anyhow::Result;
use omnivox_core::{
    punctuation::PunctuationTables,
    settings::{CapitalPitchSettings, ChunkWordLimit, SpeechDefaults},
    state::TtsState,
};
use omnivox_tts::control::{
    decode_request, ControlRequest, ControlResponse, ControlResponseEnvelope,
    PunctuationProfileDescriptor, CONTROL_PROTOCOL_VERSION,
};
use omnivox_tts::engine_configuration::LaunchSnapshot;
use omnivox_tts::host_preferences::{
    Operation, Outcome, ReloadPunctuation, ReloadResult, Scope, Status,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};

static AVAILABLE: AtomicBool = AtomicBool::new(false);
type Reporter = Arc<dyn Fn(&ControlResponseEnvelope) + Send + Sync>;
type Prepare = Box<dyn Fn() -> std::result::Result<Policy, &'static str> + Send>;

#[derive(Clone)]
struct Policy {
    tables: Arc<PunctuationTables>,
    defaults: Arc<SpeechDefaults>,
    capitals: Arc<CapitalPitchSettings>,
    chunk_words: ChunkWordLimit,
}
impl Policy {
    fn from_state(state: &TtsState) -> Self {
        Self {
            tables: Arc::clone(&state.punctuation_tables),
            defaults: Arc::clone(&state.speech_defaults),
            capitals: Arc::clone(&state.capital_pitch),
            chunk_words: state.max_chunk_words,
        }
    }
    fn from_snapshot(snapshot: &LaunchSnapshot) -> Self {
        let speech = &snapshot.resolved().speech;
        Self {
            tables: Arc::new(speech.punctuation.clone()),
            defaults: Arc::new(speech.defaults.clone()),
            capitals: Arc::new(speech.capital_pitch.clone()),
            chunk_words: speech.max_chunk_words,
        }
    }
    fn other_preferences_match(&self, other: &Self) -> bool {
        self.defaults == other.defaults
            && self.capitals == other.capitals
            && self.chunk_words == other.chunk_words
    }
    fn digest(&self) -> String {
        digest(&(
            self.tables.as_ref(),
            self.defaults.as_ref(),
            self.capitals.as_ref(),
            self.chunk_words,
        ))
    }
    fn install(self, state: &mut TtsState) {
        state.punctuation_tables = self.tables;
        state.speech_defaults = self.defaults;
        state.capital_pitch = self.capitals;
        state.max_chunk_words = self.chunk_words;
    }
}

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
    for key in [
        "punctuation",
        "speech_defaults",
        "capital_pitch",
        "speech",
        "activation_id",
        "root",
    ] {
        value.as_object_mut().unwrap().remove(key);
    }
    value
}
struct Prepared {
    policy: Policy,
    profiles: Vec<PunctuationProfileDescriptor>,
    digest: String,
    punctuation_digest: String,
    installed: mpsc::SyncSender<bool>,
}
struct Shared {
    status: Status,
    profiles: Vec<PunctuationProfileDescriptor>,
    pending: Option<Prepared>,
}
pub struct Runtime {
    shared: Arc<Mutex<Shared>>,
    tasks: Option<mpsc::SyncSender<Scope>>,
    reports: mpsc::SyncSender<ControlResponseEnvelope>,
    closed: Arc<AtomicBool>,
    advertised: bool,
}

impl Runtime {
    pub fn start(sender: mpsc::SyncSender<Input>, state: &TtsState) -> Result<Option<Self>> {
        if std::env::var_os("OMNIVOX_REMOTE_WORKER").is_some() || crate::emergency::worker() {
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
                return Err("engine, routing or output changes require restart");
            }
            Ok(Policy::from_snapshot(&candidate.snapshot))
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
        let policy = Policy::from_state(state);
        let host_hash = policy.digest();
        let shared = Arc::new(Mutex::new(Shared {
            status: Status {
                incarnation: omnivox_tts::voice_library::local::new_uuid()?,
                configuration_file: String::new(),
                startup_activation_id: startup,
                revision: 0,
                punctuation_sha256: hash.clone(),
                startup_punctuation_sha256: hash,
                host_preferences_sha256: host_hash.clone(),
                startup_host_preferences_sha256: host_hash,
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
        let mut retained = policy;
        std::thread::Builder::new()
            .name("omnivox-preference-prepare".into())
            .spawn(move || {
                while let Ok(scope) = receiver.recv() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let candidate = prepare().and_then(|policy| {
                        if scope == Scope::Punctuation && !policy.other_preferences_match(&retained)
                        {
                            Err("other host preferences changed; use host preference reload")
                        } else {
                            Ok(policy)
                        }
                    });
                    match candidate {
                        Ok(policy) => {
                            let hash = policy.digest();
                            let punctuation_digest = digest(policy.tables.as_ref());
                            let profiles = catalogue(&policy.tables);
                            let (installed, receipt) = mpsc::sync_channel(1);
                            {
                                let mut shared = worker.lock().unwrap();
                                if stopped.load(Ordering::Acquire) {
                                    break;
                                }
                                shared.status.last_operation.as_mut().unwrap().outcome =
                                    Outcome::Pending;
                                shared.pending = Some(Prepared {
                                    policy: policy.clone(),
                                    profiles,
                                    digest: hash,
                                    punctuation_digest,
                                    installed,
                                });
                            }
                            let _ = sender.try_send(Input::PreferenceWake);
                            if receipt.recv() == Ok(true) {
                                retained = policy;
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
                ControlResponse::ReloadPunctuationV1(self.begin(operation, Scope::Punctuation))
            }
            ControlRequest::ReloadHostPreferencesV1(operation) => {
                ControlResponse::ReloadHostPreferencesV1(self.begin(operation, Scope::Host))
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

    fn begin(&self, request: ReloadPunctuation, scope: Scope) -> ReloadResult {
        let mut shared = self.shared.lock().unwrap();
        let status = &mut shared.status;
        let previous = status.last_operation.as_ref();
        let last_sequence = previous.map_or(0, |op| op.sequence);
        let outcome = if request.incarnation != status.incarnation || request.sequence == 0 {
            Outcome::Stale
        } else if let Some(old) = previous.filter(|old| old.sequence == request.sequence) {
            if old.expected_revision == request.expected_revision && old.scope == scope {
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
                scope,
                sequence: request.sequence,
                expected_revision: request.expected_revision,
                outcome: Outcome::Preparing,
                reason: None,
            });
            if self.tasks.as_ref().unwrap().try_send(scope).is_err() {
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
        let outcome = if shared.status.host_preferences_sha256 == prepared.digest {
            Outcome::Unchanged
        } else if state
            .punctuation_profile
            .as_ref()
            .is_some_and(|id| !prepared.policy.tables.profiles.contains_key(id))
        {
            Outcome::Rejected
        } else if let Some(revision) = shared.status.revision.checked_add(1) {
            prepared.policy.install(state);
            shared.status.revision = revision;
            shared.status.host_preferences_sha256 = prepared.digest;
            shared.status.punctuation_sha256 = prepared.punctuation_digest;
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
    type PrepareTables =
        Box<dyn Fn() -> std::result::Result<Arc<PunctuationTables>, &'static str> + Send>;
    fn fixture(state: &TtsState, prepare: PrepareTables) -> (Runtime, mpsc::Receiver<Input>) {
        let policy = Policy::from_state(state);
        fixture_policy(
            state,
            Box::new(move || {
                prepare().map(|tables| Policy {
                    tables,
                    ..policy.clone()
                })
            }),
        )
    }
    fn fixture_policy(state: &TtsState, prepare: Prepare) -> (Runtime, mpsc::Receiver<Input>) {
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
            runtime
                .begin(request(&runtime, 1), Scope::Punctuation)
                .outcome,
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
    fn host_reload_keeps_client_choices_and_updates_one_coherent_reset_baseline() {
        let mut state = TtsState::new();
        state.current_voice = "temporary voice".into();
        state.speech_rate = 0.8;
        state.punctuation_tables = tables("old");
        state.punctuation_profile = Some("probe".into());
        let admitted = state.clone();
        let candidate = Policy {
            tables: tables("new"),
            defaults: Arc::new(SpeechDefaults {
                voice: Some("saved voice".into()),
                rate: 0.7,
                ..SpeechDefaults::default()
            }),
            capitals: Arc::new(CapitalPitchSettings {
                default: omnivox_core::settings::CapitalPitch::Off,
                ..CapitalPitchSettings::default()
            }),
            chunk_words: ChunkWordLimit::try_from(30).unwrap(),
        };
        let prepared = candidate.clone();
        let (runtime, receiver) = fixture_policy(&state, Box::new(move || Ok(prepared.clone())));
        let operation = request(&runtime, 1);
        runtime.begin(operation.clone(), Scope::Host);
        install(&runtime, &receiver, &mut state);
        assert_eq!(state.current_voice, "temporary voice");
        assert_eq!(state.speech_rate, 0.8);
        assert_eq!(state.punctuation_profile.as_deref(), Some("probe"));
        assert!(Policy::from_state(&state).other_preferences_match(&candidate));
        assert_eq!(runtime.status().host_preferences_sha256, candidate.digest());
        assert_eq!(
            runtime.begin(operation.clone(), Scope::Punctuation).outcome,
            Outcome::Stale
        );
        assert_eq!(
            runtime.begin(operation, Scope::Host).outcome,
            Outcome::Applied
        );
        assert_eq!(admitted.max_chunk_words.get(), 15);
        assert_eq!(admitted.speech_defaults.rate, 0.5);
        assert_eq!(
            admitted.punctuation_tables.all[&'!'].as_deref(),
            Some("old")
        );
        state.reset();
        assert_eq!(state.current_voice, "saved voice");
        assert_eq!(state.speech_rate, 0.7);
        assert_eq!(state.max_chunk_words.get(), 30);
        assert!(Arc::ptr_eq(&state.capital_pitch, &candidate.capitals));
        assert!(Arc::ptr_eq(&state.punctuation_tables, &candidate.tables));
    }

    #[test]
    fn punctuation_scope_compares_current_host_policy_and_rejects_mixed_edits() {
        let mut state = TtsState::new();
        let candidate = Arc::new(Mutex::new(Policy::from_state(&state)));
        candidate.lock().unwrap().chunk_words = ChunkWordLimit::try_from(30).unwrap();
        let source = Arc::clone(&candidate);
        let (runtime, receiver) =
            fixture_policy(&state, Box::new(move || Ok(source.lock().unwrap().clone())));
        runtime.begin(request(&runtime, 1), Scope::Punctuation);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while runtime.status().last_operation.as_ref().unwrap().outcome == Outcome::Preparing {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Rejected
        );
        assert_eq!(state.max_chunk_words.get(), 15);
        runtime.begin(request(&runtime, 2), Scope::Host);
        install(&runtime, &receiver, &mut state);
        candidate.lock().unwrap().tables = tables("after host reload");
        runtime.begin(request(&runtime, 3), Scope::Punctuation);
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Applied
        );
        assert_eq!(state.max_chunk_words.get(), 30);
        assert_eq!(
            state.punctuation_tables.all[&'!'].as_deref(),
            Some("after host reload")
        );
        state.punctuation_profile = Some("probe".into());
        let before = Policy::from_state(&state);
        {
            let mut next = candidate.lock().unwrap();
            next.tables = Arc::new(PunctuationTables::default());
            next.chunk_words = ChunkWordLimit::try_from(40).unwrap();
        }
        runtime.begin(request(&runtime, 4), Scope::Host);
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Rejected
        );
        assert_eq!(Policy::from_state(&state).digest(), before.digest());
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
        runtime.begin(first.clone(), Scope::Punctuation);
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.begin(first.clone(), Scope::Punctuation).outcome,
            Outcome::Applied
        );
        assert_eq!(captures.load(Ordering::SeqCst), 1);
        runtime.begin(request(&runtime, 2), Scope::Punctuation);
        install(&runtime, &receiver, &mut state);
        assert_eq!(
            runtime.status().punctuation_sha256,
            runtime.status().startup_punctuation_sha256
        );
        assert_eq!(runtime.status().revision, 2);
        assert_eq!(
            runtime.begin(first, Scope::Punctuation).outcome,
            Outcome::Stale
        );
        let mut stale = request(&runtime, 3);
        stale.incarnation = "old-worker".into();
        assert_eq!(
            runtime.begin(stale, Scope::Punctuation).outcome,
            Outcome::Stale
        );
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
        runtime.begin(request(&runtime, 1), Scope::Punctuation);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        runtime.install(&mut state, false);
        assert_eq!(
            runtime.status().last_operation.unwrap().outcome,
            Outcome::Pending
        );
        assert_eq!(
            runtime
                .begin(request(&runtime, 2), Scope::Punctuation)
                .outcome,
            Outcome::Busy
        );
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
        runtime.begin(request(&runtime, 1), Scope::Punctuation);
        enter.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..100 {
            assert_eq!(
                runtime
                    .begin(request(&runtime, 2), Scope::Punctuation)
                    .outcome,
                Outcome::Busy
            );
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
            Box::new(|| {
                Ok(Policy {
                    tables: tables("new"),
                    ..Policy::from_state(&TtsState::new())
                })
            }),
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
        runtime.begin(request(&runtime, 1), Scope::Punctuation);
        install(&runtime, &receiver, &mut state);
        assert!(Arc::ptr_eq(&tables, &state.punctuation_tables));
        assert_eq!(runtime.status().punctuation_sha256, digest(tables.as_ref()));
    }
}
