//! Runtime ownership and deterministic inventory for multiple TTS engines.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use thiserror::Error;

use crate::contracts::EngineDescriptor;
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

struct RegisteredEngine {
    engine: Arc<dyn TtsEngine>,
    descriptor: EngineDescriptor,
}

/// Engines owned by one Omnivox server session.
///
/// Entries are keyed by stable engine ID, so inventory and lookup remain
/// deterministic regardless of backend discovery order. Descriptors are
/// snapshotted explicitly and never queried while serving inventory.
#[derive(Default)]
pub struct EngineRegistry {
    generation: u64,
    entries: BTreeMap<String, RegisteredEngine>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add one engine after validating its complete descriptor.
    pub fn register(&mut self, engine: Arc<dyn TtsEngine>) -> Result<(), EngineRegistryError> {
        let descriptor = engine.descriptor();
        validate_descriptor(&descriptor)?;
        if self.entries.contains_key(&descriptor.id) {
            return Err(EngineRegistryError::DuplicateEngine {
                engine_id: descriptor.id,
            });
        }

        let engine_id = descriptor.id.clone();
        self.entries
            .insert(engine_id, RegisteredEngine { engine, descriptor });
        self.advance_generation();
        Ok(())
    }

    /// Return a shared engine handle by stable ID.
    pub fn engine(&self, engine_id: &str) -> Option<Arc<dyn TtsEngine>> {
        self.entries
            .get(engine_id)
            .map(|entry| Arc::clone(&entry.engine))
    }

    pub fn descriptor(&self, engine_id: &str) -> Option<&EngineDescriptor> {
        self.entries.get(engine_id).map(|entry| &entry.descriptor)
    }

    /// Return a stable-ID-sorted snapshot for resolution and control responses.
    pub fn inventory(&self) -> Vec<EngineDescriptor> {
        self.entries
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    /// Request cancellation from every registered engine.
    pub fn stop_all(&self) {
        for entry in self.entries.values() {
            entry.engine.stop();
        }
    }

    /// Refresh one descriptor after an explicit availability or health check.
    /// The inventory generation advances only when the descriptor changed.
    pub fn refresh_descriptor(&mut self, engine_id: &str) -> Result<bool, EngineRegistryError> {
        let engine = self
            .entries
            .get(engine_id)
            .map(|entry| Arc::clone(&entry.engine))
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

        let entry = self.entries.get_mut(engine_id).expect("entry was checked");
        if entry.descriptor == descriptor {
            return Ok(false);
        }
        entry.descriptor = descriptor;
        self.advance_generation();
        Ok(true)
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
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
}
