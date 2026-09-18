//! Connection-owned, bounded assembly of catalogue replies already requested by a client.
use super::*;
use omnivox_tts::helper_protocol::parameters::CatalogueAssembly;
use omnivox_tts::native_parameters::{CatalogueIdentity, ParameterCatalogue};
use std::collections::VecDeque;
use std::sync::Weak;

const MAX_CATALOGUES: usize = 32;
const MAX_CACHED_BYTES: usize = 1024 * 1024;
const MAX_ASSEMBLY_BYTES: usize = omnivox_tts::control::MAX_CONTROL_PAYLOAD_BYTES;

#[derive(Clone)]
pub(super) struct CachedCatalogue {
    pub(super) catalogue: Arc<ParameterCatalogue>,
    owner: Weak<dyn TtsEngine>,
    epoch: u64,
    bytes: usize,
}
impl CachedCatalogue {
    pub(super) fn current(&self, engine: &Arc<dyn TtsEngine>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, engine))
            && engine.parameter_cache_epoch() == Some(self.epoch)
    }
}
struct PartialCatalogue {
    engine_id: String,
    voice_id: Option<String>,
    owner: Weak<dyn TtsEngine>,
    epoch: u64,
    identity: CatalogueIdentity,
    assembly: CatalogueAssembly,
    bytes: usize,
}
#[derive(Default)]
pub(super) struct CatalogueCache {
    ready: VecDeque<CachedCatalogue>,
    partial: Option<PartialCatalogue>,
    bytes: usize,
}

fn same_runtime(a: &CatalogueIdentity, b: &CatalogueIdentity) -> bool {
    a.runtime_generation == b.runtime_generation
        && a.profile_id == b.profile_id
        && a.schema_id == b.schema_id
    // Catalogue revisions may differ between physical voices in one runtime.
}
impl CatalogueCache {
    pub(super) fn candidates(&self) -> Vec<CachedCatalogue> {
        self.ready.iter().cloned().collect()
    }

    fn invalidate(&mut self, query: &CatalogueQuery) {
        self.ready.retain(|entry| {
            entry.catalogue.engine_id != query.engine_id
                || entry.catalogue.voice_id != query.voice_id
        });
        self.bytes = self.ready.iter().map(|entry| entry.bytes).sum();
        if self
            .partial
            .as_ref()
            .is_some_and(|p| p.engine_id == query.engine_id && p.voice_id == query.voice_id)
        {
            self.partial = None;
        }
    }

    pub(super) fn observe(
        &mut self,
        query: &CatalogueQuery,
        response: &ControlResponse,
        engine: &Arc<dyn TtsEngine>,
        epoch: Option<u64>,
    ) {
        if matches!(
            response,
            ControlResponse::EngineParametersV1 {
                result: CatalogueResult::Busy { .. },
                ..
            }
        ) {
            return;
        }
        let Some(epoch) =
            epoch.filter(|epoch| *epoch > 0 && engine.parameter_cache_epoch() == Some(*epoch))
        else {
            self.invalidate(query);
            return;
        };
        let ControlResponse::EngineParametersV1 {
            engine_id,
            result: page @ CatalogueResult::Ready { identity, .. },
        } = response
        else {
            self.invalidate(query);
            return;
        };
        if engine_id != &query.engine_id {
            self.invalidate(query);
            return;
        }
        let bytes = match serde_json::to_vec(page) {
            Ok(v) => v.len(),
            Err(_) => {
                self.invalidate(query);
                return;
            }
        };
        // One engine cannot contribute mixed runtime/profile/schema observations.
        self.ready.retain(|entry| {
            entry.catalogue.engine_id != query.engine_id
                || (entry.epoch == epoch
                    && entry.current(engine)
                    && same_runtime(&entry.catalogue.identity, identity))
        });
        self.bytes = self.ready.iter().map(|entry| entry.bytes).sum();
        if query.cursor.is_none() {
            self.invalidate(query);
            self.partial =
                CatalogueAssembly::new(query.clone())
                    .ok()
                    .map(|assembly| PartialCatalogue {
                        engine_id: query.engine_id.clone(),
                        voice_id: query.voice_id.clone(),
                        owner: Arc::downgrade(engine),
                        epoch,
                        identity: identity.clone(),
                        assembly,
                        bytes: 0,
                    });
        }
        let Some(mut partial) = self.partial.take() else {
            return;
        };
        // An orphan continuation cannot overwrite or complete another sequence.
        if partial.engine_id != query.engine_id || partial.voice_id != query.voice_id {
            self.partial = Some(partial);
            return;
        }
        if partial.epoch != epoch
            || partial.identity != *identity
            || !partial
                .owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, engine))
            || partial.bytes.saturating_add(bytes) > MAX_ASSEMBLY_BYTES
            || partial.assembly.push(query, page).is_err()
        {
            return;
        }
        partial.bytes += bytes;
        if partial.assembly.next_query().is_some() {
            self.partial = Some(partial);
            return;
        }
        let Ok(catalogue) = partial.assembly.finish() else {
            return;
        };
        let Ok(encoded) = serde_json::to_vec(&catalogue) else {
            return;
        };
        let bytes = encoded.len();
        if bytes > MAX_ASSEMBLY_BYTES {
            return;
        }
        self.invalidate(query);
        while self.ready.len() >= MAX_CATALOGUES
            || self.bytes.saturating_add(bytes) > MAX_CACHED_BYTES
        {
            let Some(oldest) = self.ready.pop_front() else {
                return;
            };
            self.bytes -= oldest.bytes;
        }
        self.bytes += bytes;
        self.ready.push_back(CachedCatalogue {
            catalogue: Arc::new(catalogue),
            owner: Arc::downgrade(engine),
            epoch,
            bytes,
        });
    }
}

#[cfg(test)]
mod tests;
