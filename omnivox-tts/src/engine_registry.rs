//! Runtime ownership and deterministic inventory for multiple TTS engines.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use thiserror::Error;

use crate::contracts::{Availability, EngineDescriptor, EngineHealth};
use crate::TtsEngine;

/// Registry mutations that leave the previous inventory untouched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineRegistryError {
    #[error("engine descriptor has an empty ID")]
    EmptyEngineId,

    #[error("engine {engine_id} is already registered")]
    DuplicateEngine { engine_id: String },

    #[error("engine {engine_id} describes a voice owned by {voice_engine_id}")]
    ForeignVoice {
        engine_id: String,
        voice_engine_id: String,
    },

    #[error("engine {engine_id} repeats physical voice ID {voice_id}")]
    DuplicateVoice { engine_id: String, voice_id: String },

    #[error("engine {engine_id} names missing default voice {voice_id}")]
    MissingDefaultVoice { engine_id: String, voice_id: String },

    #[error("engine {engine_id} is not registered")]
    UnknownEngine { engine_id: String },

    #[error("engine {expected} changed its descriptor ID to {received}")]
    ChangedEngineId { expected: String, received: String },
}

type EngineFactory = Arc<dyn Fn() -> Result<Arc<dyn TtsEngine>, String> + Send + Sync>;

struct RegisteredEngine {
    engine: Option<Arc<dyn TtsEngine>>,
    descriptor: EngineDescriptor,
    retry: Option<EngineFactory>,
    rescanning: bool,
}

#[derive(Default)]
struct RegistryState {
    generation: u64,
    entries: BTreeMap<String, RegisteredEngine>,
}

impl RegistryState {
    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
}

/// Engines owned by one Omnivox server session.
///
/// Inventory reads use cached descriptors and never call an engine. A failed
/// startup can be retried off the command thread; its validated descriptor and
/// handle become visible together under the inventory lock.
#[derive(Default)]
pub struct EngineRegistry {
    inner: Arc<RwLock<RegistryState>>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.inner.read().unwrap().generation
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add one engine after validating its complete descriptor.
    pub fn register(&mut self, engine: Arc<dyn TtsEngine>) -> Result<(), EngineRegistryError> {
        let descriptor = engine.descriptor();
        self.insert(RegisteredEngine {
            engine: Some(engine),
            descriptor,
            retry: None,
            rescanning: false,
        })
    }

    /// Retain a failed configured engine without inventing voices or opening it
    /// during inventory reads. The factory must bound its own startup work.
    pub fn register_unavailable(
        &mut self,
        descriptor: EngineDescriptor,
        retry: impl Fn() -> Result<Arc<dyn TtsEngine>, String> + Send + Sync + 'static,
    ) -> Result<(), EngineRegistryError> {
        self.insert(RegisteredEngine {
            engine: None,
            descriptor,
            retry: Some(Arc::new(retry)),
            rescanning: false,
        })
    }

    fn insert(&mut self, entry: RegisteredEngine) -> Result<(), EngineRegistryError> {
        validate_descriptor(&entry.descriptor)?;
        let mut inner = self.inner.write().unwrap();
        if inner.entries.contains_key(&entry.descriptor.id) {
            return Err(EngineRegistryError::DuplicateEngine {
                engine_id: entry.descriptor.id,
            });
        }
        inner.entries.insert(entry.descriptor.id.clone(), entry);
        inner.advance_generation();
        Ok(())
    }

    /// Return a shared engine handle, or None for an unavailable startup entry.
    pub fn engine(&self, engine_id: &str) -> Option<Arc<dyn TtsEngine>> {
        self.inner
            .read()
            .unwrap()
            .entries
            .get(engine_id)
            .and_then(|entry| entry.engine.clone())
    }

    pub fn descriptor(&self, engine_id: &str) -> Option<EngineDescriptor> {
        self.inner
            .read()
            .unwrap()
            .entries
            .get(engine_id)
            .map(|entry| entry.descriptor.clone())
    }

    /// Return a generation and stable-ID-sorted inventory from the same read.
    pub fn snapshot(&self) -> (u64, Vec<EngineDescriptor>) {
        let inner = self.inner.read().unwrap();
        (
            inner.generation,
            inner
                .entries
                .values()
                .map(|entry| entry.descriptor.clone())
                .collect(),
        )
    }

    pub fn inventory(&self) -> Vec<EngineDescriptor> {
        self.snapshot().1
    }

    /// Retry one startup failure asynchronously, with at most one retry per
    /// engine in flight. Success enables it for subsequent routing snapshots.
    pub fn request_rescan(&self, engine_id: &str) -> Result<(), String> {
        let retry = {
            let mut inner = self.inner.write().unwrap();
            let entry = inner
                .entries
                .get_mut(engine_id)
                .ok_or_else(|| format!("unknown engine {engine_id}"))?;
            if entry.rescanning {
                return Err(format!("engine {engine_id} rescan is already in progress"));
            }
            let retry = entry
                .retry
                .clone()
                .ok_or_else(|| format!("engine {engine_id} has no startup failure to rescan"))?;
            entry.rescanning = true;
            entry.descriptor.availability = Availability::Unavailable {
                reason: "Runtime rescan in progress".to_owned(),
            };
            inner.advance_generation();
            retry
        };
        let inner = Arc::clone(&self.inner);
        let id = engine_id.to_owned();
        let spawn = std::thread::Builder::new()
            .name(format!("omnivox-{engine_id}-rescan"))
            .spawn(move || {
                // A constructor panic must not poison the registry or leave the
                // entry permanently marked as rescanning.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let engine = retry()?;
                    let descriptor = engine.descriptor();
                    validate_descriptor(&descriptor).map_err(|error| error.to_string())?;
                    if descriptor.id != id || !descriptor.can_synthesize() {
                        return Err(
                            "Rescanned engine returned an unavailable or mismatched descriptor"
                                .to_owned(),
                        );
                    }
                    Ok((engine, descriptor))
                }))
                .unwrap_or_else(|_| Err("Engine rescan panicked".to_owned()));
                Self::finish_rescan(&inner, &id, result);
            });
        if let Err(error) = spawn {
            let reason = format!("Could not start engine rescan: {error}");
            Self::finish_rescan(&self.inner, engine_id, Err(reason.clone()));
            return Err(reason);
        }
        Ok(())
    }

    fn finish_rescan(
        inner: &RwLock<RegistryState>,
        engine_id: &str,
        result: Result<(Arc<dyn TtsEngine>, EngineDescriptor), String>,
    ) {
        let mut inner = inner.write().unwrap();
        let entry = inner
            .entries
            .get_mut(engine_id)
            .expect("rescan entry remains registered");
        entry.rescanning = false;
        match result {
            Ok((engine, descriptor)) => {
                entry.engine = Some(engine);
                entry.descriptor = descriptor;
                entry.retry = None;
            }
            Err(reason) => {
                entry.descriptor.availability = Availability::Unavailable {
                    reason: reason.clone(),
                };
                entry.descriptor.health = EngineHealth::Failed { reason };
            }
        }
        inner.advance_generation();
    }

    /// Request cancellation without holding an inventory lock across engines.
    pub fn stop_all(&self) {
        let engines: Vec<_> = self
            .inner
            .read()
            .unwrap()
            .entries
            .values()
            .filter_map(|entry| entry.engine.clone())
            .collect();
        for engine in engines {
            engine.stop();
        }
    }

    /// Refresh one descriptor after an explicit availability or health check.
    /// The inventory generation advances only when the descriptor changed.
    pub fn refresh_descriptor(&mut self, engine_id: &str) -> Result<bool, EngineRegistryError> {
        let engine = self
            .engine(engine_id)
            .ok_or_else(|| EngineRegistryError::UnknownEngine {
                engine_id: engine_id.to_owned(),
            })?;
        let descriptor = engine.descriptor();
        if descriptor.id != engine_id {
            return Err(EngineRegistryError::ChangedEngineId {
                expected: engine_id.to_owned(),
                received: descriptor.id,
            });
        }
        validate_descriptor(&descriptor)?;
        let mut inner = self.inner.write().unwrap();
        let entry = inner.entries.get_mut(engine_id).expect("entry was checked");
        if entry.descriptor == descriptor {
            return Ok(false);
        }
        entry.descriptor = descriptor;
        inner.advance_generation();
        Ok(true)
    }
}

pub(crate) fn validate_descriptor(
    descriptor: &EngineDescriptor,
) -> Result<(), EngineRegistryError> {
    if descriptor.id.is_empty() {
        return Err(EngineRegistryError::EmptyEngineId);
    }

    let mut voice_ids = HashSet::with_capacity(descriptor.voices.len());
    for voice in &descriptor.voices {
        if voice.id.engine_id != descriptor.id {
            return Err(EngineRegistryError::ForeignVoice {
                engine_id: descriptor.id.clone(),
                voice_engine_id: voice.id.engine_id.clone(),
            });
        }
        if !voice_ids.insert(voice.id.voice_id.as_str()) {
            return Err(EngineRegistryError::DuplicateVoice {
                engine_id: descriptor.id.clone(),
                voice_id: voice.id.voice_id.clone(),
            });
        }
    }

    if let Some(default_voice_id) = &descriptor.default_voice_id {
        if !voice_ids.contains(default_voice_id.as_str()) {
            return Err(EngineRegistryError::MissingDefaultVoice {
                engine_id: descriptor.id.clone(),
                voice_id: default_voice_id.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::contracts::{
        AcssCapabilities, AudioOutputMode, Availability, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, EngineHealth, MarkerCapabilities, PhysicalVoiceId, VoiceDescriptor,
    };
    use crate::{
        AudioBuffer, SynthesisRequest, SynthesisResult, TtsError, VoiceInfo, VoiceQuality,
    };

    struct MockEngine {
        descriptor: Mutex<EngineDescriptor>,
        stop_count: AtomicUsize,
    }

    impl MockEngine {
        fn new(engine_id: &str) -> Self {
            Self {
                descriptor: Mutex::new(descriptor(engine_id)),
                stop_count: AtomicUsize::new(0),
            }
        }

        fn set_health(&self, health: EngineHealth) {
            self.descriptor.lock().unwrap().health = health;
        }
    }

    impl TtsEngine for MockEngine {
        fn descriptor(&self) -> EngineDescriptor {
            self.descriptor.lock().unwrap().clone()
        }

        fn synthesize(&self, _request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            Ok(SynthesisResult::audio(
                self.descriptor().id,
                None,
                AudioBuffer::empty(),
            ))
        }

        fn stop(&self) {
            self.stop_count.fetch_add(1, Ordering::Relaxed);
        }

        fn is_speaking(&self) -> bool {
            false
        }

        fn available_voices(&self) -> Vec<VoiceInfo> {
            Vec::new()
        }

        fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
            None
        }
    }

    fn descriptor(engine_id: &str) -> EngineDescriptor {
        let voice_id = format!("{engine_id}:default");
        EngineDescriptor {
            id: engine_id.to_owned(),
            display_name: engine_id.to_owned(),
            version: None,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: EngineCapabilities {
                acss: AcssCapabilities::default(),
                audio_output: AudioOutputMode::BufferedPcm,
                cancellation: CancellationSupport::PlaybackOnly,
                concurrency: ConcurrencyModel::Serialized,
                markers: MarkerCapabilities::default(),
                language_switching: false,
                text_repertoire: crate::contracts::TextRepertoire::Unicode,
                post_synthesis_dimensions: Vec::new(),
                native_extensions: Vec::new(),
            },
            voices: vec![VoiceDescriptor {
                id: PhysicalVoiceId::new(engine_id, &voice_id),
                display_name: "Default".to_owned(),
                language: None,
                gender: None,
                quality: VoiceQuality::Compact,
                availability: Availability::Available,
            }],
            default_voice_id: Some(voice_id),
        }
    }

    #[test]
    fn registry_inventory_is_sorted_by_stable_engine_id() {
        let mut registry = EngineRegistry::new();
        registry
            .register(Arc::new(MockEngine::new("winrt")))
            .unwrap();
        registry
            .register(Arc::new(MockEngine::new("espeak")))
            .unwrap();

        let ids: Vec<_> = registry
            .inventory()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();

        assert_eq!(ids, ["espeak", "winrt"]);
        assert_eq!(registry.generation(), 2);
    }

    #[test]
    fn duplicate_engine_is_rejected_without_mutation() {
        let mut registry = EngineRegistry::new();
        registry
            .register(Arc::new(MockEngine::new("winrt")))
            .unwrap();

        let error = registry
            .register(Arc::new(MockEngine::new("winrt")))
            .unwrap_err();

        assert!(matches!(error, EngineRegistryError::DuplicateEngine { .. }));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.generation(), 1);
    }

    #[test]
    fn invalid_descriptor_is_rejected_atomically() {
        let engine = Arc::new(MockEngine::new("winrt"));
        engine.descriptor.lock().unwrap().voices[0].id.engine_id = "other".to_owned();
        let mut registry = EngineRegistry::new();

        let error = registry.register(engine).unwrap_err();

        assert!(matches!(error, EngineRegistryError::ForeignVoice { .. }));
        assert!(registry.is_empty());
        assert_eq!(registry.generation(), 0);
    }

    #[test]
    fn lookup_returns_the_registered_engine() {
        let engine: Arc<dyn TtsEngine> = Arc::new(MockEngine::new("winrt"));
        let mut registry = EngineRegistry::new();
        registry.register(Arc::clone(&engine)).unwrap();

        let found = registry.engine("winrt").unwrap();

        assert!(Arc::ptr_eq(&engine, &found));
        assert!(registry.engine("missing").is_none());
    }

    #[test]
    fn stop_all_requests_cancellation_from_every_engine() {
        let winrt = Arc::new(MockEngine::new("winrt"));
        let espeak = Arc::new(MockEngine::new("espeak"));
        let mut registry = EngineRegistry::new();
        registry
            .register(Arc::clone(&winrt) as Arc<dyn TtsEngine>)
            .unwrap();
        registry
            .register(Arc::clone(&espeak) as Arc<dyn TtsEngine>)
            .unwrap();

        registry.stop_all();

        assert_eq!(winrt.stop_count.load(Ordering::Relaxed), 1);
        assert_eq!(espeak.stop_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn descriptor_refresh_advances_only_after_a_change() {
        let engine = Arc::new(MockEngine::new("winrt"));
        let mut registry = EngineRegistry::new();
        registry
            .register(Arc::clone(&engine) as Arc<dyn TtsEngine>)
            .unwrap();

        assert!(!registry.refresh_descriptor("winrt").unwrap());
        assert_eq!(registry.generation(), 1);

        engine.set_health(EngineHealth::Degraded {
            reason: "recovering".to_owned(),
        });
        assert!(registry.refresh_descriptor("winrt").unwrap());
        assert_eq!(registry.generation(), 2);
        assert!(matches!(
            registry.descriptor("winrt").unwrap().health,
            EngineHealth::Degraded { .. }
        ));
    }

    fn wait_for_rescan(registry: &EngineRegistry, generation: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while registry.generation() < generation {
            assert!(
                std::time::Instant::now() < deadline,
                "rescan did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn startup_rescan_keeps_inventory_and_fallback_responsive() {
        let mut registry = EngineRegistry::new();
        let fallback = Arc::new(MockEngine::new("espeak"));
        registry.register(fallback.clone()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let (release, wait) = std::sync::mpsc::channel();
        let wait = Mutex::new(wait);
        registry
            .register_unavailable(
                EngineDescriptor::unavailable("dectalk", "dictionary missing"),
                move || {
                    worker_calls.fetch_add(1, Ordering::SeqCst);
                    wait.lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap();
                    Ok(Arc::new(MockEngine::new("dectalk")))
                },
            )
            .unwrap();
        let (generation, before) = registry.snapshot();
        assert_eq!(generation, 2);
        assert!(before[0].voices.is_empty());
        assert!(!before[0].can_synthesize());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(registry.engine("dectalk").is_none());

        registry.request_rescan("dectalk").unwrap();
        assert!(registry
            .request_rescan("dectalk")
            .unwrap_err()
            .contains("in progress"));
        let (generation, during) = registry.snapshot();
        assert_eq!(generation, 3);
        assert!(!during[0].can_synthesize());
        assert!(registry
            .engine("espeak")
            .unwrap()
            .synthesize(&SynthesisRequest::new(
                "fallback still works",
                Default::default()
            ))
            .is_ok());
        registry.stop_all();
        assert_eq!(fallback.stop_count.load(Ordering::Relaxed), 1);
        release.send(()).unwrap();
        wait_for_rescan(&registry, 4);
        let (generation, after) = registry.snapshot();
        assert_eq!(generation, 4);
        assert!(after[0].can_synthesize());
        assert!(!after[0].voices.is_empty());
        assert!(registry.engine("dectalk").is_some());
        assert!(registry.request_rescan("dectalk").is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            !before[0].can_synthesize(),
            "prior snapshots must stay unchanged"
        );
    }

    #[test]
    fn failed_panicking_and_mismatched_rescans_remain_retryable() {
        let mut registry = EngineRegistry::new();
        let attempts = AtomicUsize::new(0);
        registry
            .register_unavailable(
                EngineDescriptor::unavailable("dectalk", "missing DLL"),
                move || match attempts.fetch_add(1, Ordering::SeqCst) {
                    0 => Err("dictionary still missing".to_owned()),
                    1 => panic!("broken constructor"),
                    2 => Ok(Arc::new(MockEngine::new("wrong-engine"))),
                    _ => Ok(Arc::new(MockEngine::new("dectalk"))),
                },
            )
            .unwrap();
        assert!(registry.request_rescan("unknown").is_err());
        for (index, reason) in ["dictionary still missing", "panicked", "mismatched"]
            .iter()
            .enumerate()
        {
            registry.request_rescan("dectalk").unwrap();
            wait_for_rescan(&registry, 3 + index as u64 * 2);
            assert!(registry.engine("dectalk").is_none());
            assert!(
                matches!(registry.descriptor("dectalk").unwrap().availability,
                Availability::Unavailable { reason: actual } if actual.contains(reason))
            );
        }
        registry.request_rescan("dectalk").unwrap();
        wait_for_rescan(&registry, 9);
        assert!(registry.engine("dectalk").is_some());
    }
}
