//! Immutable administrative voice eligibility for one server generation.
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use super::RuntimeLibrary;
use crate::contracts::{Availability, EngineDescriptor, PhysicalVoiceId};
use crate::{
    SynthesisRequest, SynthesisResult, SynthesisStreamCompletion, SynthesisStreamSink, TtsEngine,
    TtsError, VoiceInfo,
};

const EXCLUDED: &str = "excluded by voice-library configuration";

/// Explicit native file settings replace only that provider's managed load set.
/// They never override global physical-voice exclusions.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderOverrides {
    pub piper: bool,
    pub flite: bool,
}

/// Selection policy derived from structurally validated metadata. This does not
/// verify assets, initialize providers or establish their runtime health.
#[derive(Debug, Clone, Default)]
pub struct VoiceEligibility {
    disabled: HashSet<PhysicalVoiceId>,
    managed: BTreeMap<String, Vec<String>>,
    overridden: Vec<String>,
}

impl VoiceEligibility {
    pub fn from_library(library: &RuntimeLibrary, overrides: ProviderOverrides) -> Self {
        let document = library.document();
        let mut policy = Self {
            disabled: document.disabled_physical_ids.iter().cloned().collect(),
            ..Self::default()
        };
        if let Some(piper) = &document.piper {
            if overrides.piper {
                policy.overridden.push("piper".to_owned());
            } else {
                policy.managed.insert(
                    "piper".to_owned(),
                    piper
                        .models
                        .iter()
                        .flat_map(|model| {
                            model.voices.iter().map(|voice| voice.physical_id.clone())
                        })
                        .collect(),
                );
            }
        }
        if let Some(flite) = &document.flite {
            if overrides.flite {
                policy.overridden.push("flite".to_owned());
            } else {
                let voices = flite
                    .builtin_slt
                    .then(|| "cmu_us_slt".to_owned())
                    .into_iter()
                    .chain(flite.files.iter().map(|voice| voice.physical_id.clone()))
                    .collect();
                policy.managed.insert("flite".to_owned(), voices);
            }
        }
        if let Some(mbrola) = &document.mbrola {
            policy.managed.insert(
                "mbrola".to_owned(),
                mbrola.voice_ids().map(str::to_owned).collect(),
            );
        }
        if let Some(rhvoice) = &document.rhvoice {
            if !rhvoice.inherit_external {
                policy.managed.insert(
                    "rhvoice".into(),
                    rhvoice
                        .voices
                        .iter()
                        .map(|v| v.physical_id.clone())
                        .collect(),
                );
            }
        }
        policy.overridden.sort();
        policy
    }

    pub fn overridden_engines(&self) -> &[String] {
        &self.overridden
    }

    pub fn permits(&self, voice: &PhysicalVoiceId) -> bool {
        !self.disabled.contains(voice)
            && self
                .managed
                .get(&voice.engine_id)
                .is_none_or(|voices| voices.contains(&voice.voice_id))
    }

    /// An explicit empty load set requires no helper and cannot be rescanned.
    pub fn excludes_provider(&self, engine_id: &str) -> bool {
        self.managed.get(engine_id).is_some_and(Vec::is_empty)
    }

    /// Preserve excluded rows for saved palette references while replacing
    /// defaults that could bypass eligibility. Native failures remain intact.
    pub fn project_descriptor(&self, mut descriptor: EngineDescriptor) -> EngineDescriptor {
        // Preserve explicit exclusions of combinations absent from the flat inventory.
        let mut excluded: Vec<_> = self
            .disabled
            .iter()
            .filter(|id| id.engine_id == descriptor.id)
            .collect();
        excluded.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        for id in excluded {
            if !descriptor.voices.iter().any(|voice| voice.id == *id) {
                if let Some(voice) = descriptor.voice(&id.voice_id) {
                    descriptor.voices.push(voice);
                }
            }
        }
        let all_excluded = self.excludes_provider(&descriptor.id)
            || (!descriptor.voices.is_empty()
                && descriptor
                    .voices
                    .iter()
                    .all(|voice| !self.permits(&voice.id)));
        for voice in &mut descriptor.voices {
            if !self.permits(&voice.id) {
                voice.availability = Availability::Unavailable {
                    reason: EXCLUDED.to_owned(),
                };
            }
        }
        let available = |id: &str| {
            descriptor
                .voices
                .iter()
                .any(|voice| voice.id.voice_id == id && voice.availability.is_available())
        };
        descriptor.default_voice_id = if let Some(order) = self.managed.get(&descriptor.id) {
            order.iter().find(|id| available(id)).cloned()
        } else {
            match descriptor.default_voice_id {
                Some(id) if available(&id) => Some(id),
                Some(_) => descriptor
                    .voices
                    .iter()
                    .find(|voice| voice.availability.is_available())
                    .map(|voice| voice.id.voice_id.clone()),
                None => None,
            }
        };
        if all_excluded {
            descriptor.availability = Availability::Unavailable {
                reason: EXCLUDED.to_owned(),
            };
            descriptor.default_voice_id = None;
        }
        descriptor
    }

    /// Administrative eligibility from one inventory snapshot. Runtime health
    /// remains a separate inventory field; disabled engines contribute no IDs.
    pub fn eligible_voices(
        &self,
        inventory: &[EngineDescriptor],
        disabled_engines: &[String],
    ) -> Vec<PhysicalVoiceId> {
        let mut voices: Vec<_> = inventory
            .iter()
            .filter(|engine| !disabled_engines.contains(&engine.id))
            .flat_map(|engine| engine.voices.iter())
            .filter(|voice| self.permits(&voice.id))
            .map(|voice| voice.id.clone())
            .collect();
        voices.sort_by(|a, b| (&a.engine_id, &a.voice_id).cmp(&(&b.engine_id, &b.voice_id)));
        voices.dedup();
        voices
    }

    /// Guard direct synthesis as well as inventory-based selectors. Callers
    /// still configure the native load set before constructing an engine.
    pub fn guard_engine(self: &Arc<Self>, engine: Arc<dyn TtsEngine>) -> Arc<dyn TtsEngine> {
        Arc::new(EligibleEngine {
            engine_id: engine.descriptor().id,
            engine,
            policy: Arc::clone(self),
        })
    }
}

struct EligibleEngine {
    engine_id: String,
    engine: Arc<dyn TtsEngine>,
    policy: Arc<VoiceEligibility>,
}

impl EligibleEngine {
    fn admit(&self, request: &SynthesisRequest) -> Result<(), TtsError> {
        let id = request.voice_id_for_engine(&self.engine_id)?;
        // Only exact advertised physical IDs cross the managed boundary.
        // Native/display-name aliases cannot reopen an excluded voice.
        if !self
            .policy
            .permits(&PhysicalVoiceId::new(&self.engine_id, id))
        {
            return Err(TtsError::VoiceNotFound(EXCLUDED.to_owned()));
        }
        let descriptor = self.policy.project_descriptor(self.engine.descriptor());
        match descriptor.voice(id) {
            Some(voice) => match &voice.availability {
                Availability::Available => Ok(()),
                Availability::Unavailable { reason } => {
                    Err(TtsError::VoiceNotFound(reason.clone()))
                }
            },
            None => Err(TtsError::VoiceNotFound(
                "voice is not advertised by this engine".to_owned(),
            )),
        }
    }
}

impl TtsEngine for EligibleEngine {
    fn engine_parameters(
        &self,
        query: crate::engine_parameters::CatalogueQuery,
    ) -> Result<crate::engine_parameters::CatalogueResult, crate::engine_parameters::CatalogueError>
    {
        use crate::engine_parameters::{unavailable, validate_query, CatalogueUnavailable};
        validate_query(&query)?;
        if self.policy.excludes_provider(&self.engine_id) {
            return Ok(unavailable(
                CatalogueUnavailable::EngineUnavailable,
                EXCLUDED,
            ));
        }
        if query.voice_id.as_ref().is_some_and(|id| {
            !self
                .policy
                .permits(&PhysicalVoiceId::new(&self.engine_id, id))
        }) {
            return Ok(unavailable(
                CatalogueUnavailable::VoiceUnavailable,
                EXCLUDED,
            ));
        }
        self.engine.engine_parameters(query)
    }

    fn descriptor(&self) -> EngineDescriptor {
        self.policy.project_descriptor(self.engine.descriptor())
    }
    fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
        let descriptor = self.engine.descriptor();
        if self.policy.excludes_provider(&self.engine_id)
            || (!descriptor.voices.is_empty()
                && descriptor
                    .voices
                    .iter()
                    .all(|voice| !self.policy.permits(&voice.id)))
        {
            return Err(TtsError::VoiceNotFound(EXCLUDED.to_owned()));
        }
        self.engine.prepare_recovery_probe()
    }
    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        self.admit(request)?;
        self.engine.synthesize(request)
    }
    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn SynthesisStreamSink,
    ) -> Result<SynthesisStreamCompletion, TtsError> {
        self.admit(request)?;
        self.engine.synthesize_stream(request, sink)
    }
    fn stop(&self) {
        self.engine.stop();
    }
    fn is_speaking(&self) -> bool {
        self.engine.is_speaking()
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor()
            .voices
            .into_iter()
            .filter(|voice| voice.availability.is_available())
            .map(|voice| VoiceInfo {
                identifier: voice.id.voice_id,
                name: voice.display_name,
                language: voice.language.unwrap_or_else(|| "und".to_owned()),
                quality: voice.quality,
            })
            .collect()
    }
    fn voice_info(&self, id: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|voice| voice.identifier == id)
    }
}

#[cfg(test)]
mod tests;
