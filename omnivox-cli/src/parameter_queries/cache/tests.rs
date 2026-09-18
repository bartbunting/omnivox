use super::*;
use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::{SynthesisRequest, SynthesisResult, TtsError, VoiceInfo};
use serde_json::Value;
use std::sync::atomic::AtomicU64;
struct Engine(AtomicU64);
impl TtsEngine for Engine {
    fn parameter_cache_epoch(&self) -> Option<u64> {
        Some(self.0.load(Ordering::Acquire))
    }
    fn descriptor(&self) -> EngineDescriptor {
        panic!("cache must not discover engines")
    }
    fn synthesize(&self, _: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        panic!("cache must not speak")
    }
    fn stop(&self) {
        panic!("cache must not stop speech")
    }
    fn is_speaking(&self) -> bool {
        false
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        panic!("cache must not load voices")
    }
    fn voice_info(&self, _: &str) -> Option<VoiceInfo> {
        panic!("cache must not load voices")
    }
}
fn fake_engine() -> Arc<dyn TtsEngine> {
    Arc::new(Engine(AtomicU64::new(1)))
}
fn page() -> CatalogueResult {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../docs/protocol-fixtures/engine-voice-parameters.json"
    ))
    .unwrap();
    serde_json::from_value(fixture["messages"]["catalogue_response"]["result"].clone()).unwrap()
}
fn query() -> CatalogueQuery {
    CatalogueQuery {
        engine_id: "dectalk".into(),
        voice_id: Some("paul".into()),
        cursor: None,
        expected_catalogue_revision: None,
    }
}
fn accept(
    cache: &mut CatalogueCache,
    q: &CatalogueQuery,
    page: CatalogueResult,
    engine: &Arc<dyn TtsEngine>,
) {
    let response = checked_result(q, Ok(page));
    cache.observe(q, &response, engine, Some(1));
}
fn pages() -> (CatalogueResult, CatalogueQuery, CatalogueResult) {
    let mut first = page();
    let mut last = page();
    let CatalogueResult::Ready {
        parameters,
        next_cursor,
        identity,
        ..
    } = &mut first
    else {
        panic!()
    };
    // Independent opaque cursor accepted by the typed transport.
    *next_cursor = Some("1".into());
    let q = CatalogueQuery {
        cursor: next_cursor.clone(),
        expected_catalogue_revision: Some(identity.catalogue_revision.clone()),
        ..query()
    };
    let tail = parameters.split_off(1);
    let CatalogueResult::Ready { parameters, .. } = &mut last else {
        panic!()
    };
    *parameters = tail;
    (first, q, last)
}
#[test]
fn only_complete_validated_pages_are_published_and_busy_does_not_destroy_assembly() {
    let engine = fake_engine();
    let mut cache = CatalogueCache::default();
    let (first, next, last) = pages();
    accept(&mut cache, &query(), first, &engine);
    assert!(cache.candidates().is_empty());
    assert!(cache.partial.is_some());
    cache.observe(
        &next,
        &catalogue(&next, CatalogueResult::Busy { retry_after_ms: 50 }),
        &engine,
        Some(1),
    );
    accept(&mut cache, &next, last, &engine);
    assert_eq!(cache.candidates().len(), 1);
    assert!(cache.partial.is_none());
    let c = &cache.ready[0];
    assert!(c.current(&engine));
    assert_eq!(c.catalogue.parameters.len(), 2);
}
#[test]
fn discontinuous_pages_and_duplicate_parameters_never_publish_partial_metadata() {
    for fault in 0..4 {
        let engine = fake_engine();
        let mut cache = CatalogueCache::default();
        let (first, mut next, mut last) = pages();
        accept(&mut cache, &query(), first.clone(), &engine);
        match fault {
            0 => next.cursor = Some("9".into()),
            1 => {
                let CatalogueResult::Ready { identity, .. } = &mut last else {
                    panic!()
                };
                identity.runtime_generation += 1;
            }
            2 => {
                let CatalogueResult::Ready { parameters, .. } = &mut last else {
                    panic!()
                };
                let CatalogueResult::Ready { parameters: p, .. } = &first else {
                    panic!()
                };
                parameters[0] = p[0].clone();
            }
            _ => {
                let CatalogueResult::Ready { mappings, .. } = &mut last else {
                    panic!()
                };
                mappings.clear();
            }
        }
        accept(&mut cache, &next, last, &engine);
        assert!(cache.ready.is_empty(), "fault {fault}");
        assert!(cache.partial.is_none());
    }
    let mut cache = CatalogueCache::default();
    let (_, q, last) = pages();
    accept(&mut cache, &q, last, &fake_engine());
    assert!(cache.ready.is_empty());
}
#[test]
fn replacement_owner_runtime_and_profile_cannot_reuse_saved_metadata() {
    let engine = fake_engine();
    let replacement = fake_engine();
    let mut cache = CatalogueCache::default();
    accept(&mut cache, &query(), page(), &engine);
    let frozen = cache.candidates();
    assert!(!frozen[0].current(&replacement));
    let mut other = page();
    let CatalogueResult::Ready {
        voice_id, identity, ..
    } = &mut other
    else {
        panic!()
    };
    *voice_id = Some("betty".into());
    identity.runtime_generation += 1;
    identity.profile_id = "updated.v1".into();
    let q = CatalogueQuery {
        voice_id: Some("betty".into()),
        ..query()
    };
    accept(&mut cache, &q, other, &engine);
    assert_eq!(cache.ready.len(), 1);
    assert_eq!(cache.ready[0].catalogue.voice_id.as_deref(), Some("betty"));
    assert_eq!(frozen[0].catalogue.voice_id.as_deref(), Some("paul"));
}
#[test]
fn count_budget_evicts_old_catalogues_without_retaining_engines() {
    let engine = fake_engine();
    let mut cache = CatalogueCache::default();
    for i in 0..MAX_CATALOGUES + 1 {
        let mut page = page();
        let CatalogueResult::Ready { voice_id, .. } = &mut page else {
            panic!()
        };
        *voice_id = Some(format!("voice{i}"));
        let q = CatalogueQuery {
            voice_id: voice_id.clone(),
            ..query()
        };
        accept(&mut cache, &q, page, &engine);
    }
    assert_eq!(cache.ready.len(), MAX_CATALOGUES);
    assert!(cache.bytes <= MAX_CACHED_BYTES);
    assert!(cache
        .ready
        .iter()
        .all(|e| e.catalogue.voice_id.as_deref() != Some("voice0")));
    assert_eq!(Arc::strong_count(&engine), 1);
    let entries = cache.candidates();
    drop(engine);
    assert!(entries[0].owner.upgrade().is_none());
}
#[test]
fn oversized_assembly_and_terminal_errors_leave_no_positive_metadata() {
    let engine = fake_engine();
    let mut cache = CatalogueCache::default();
    for index in 0..4 {
        let mut page = large_page(index * 64);
        let CatalogueResult::Ready {
            next_cursor,
            identity,
            ..
        } = &mut page
        else {
            panic!()
        };
        if index < 3 {
            *next_cursor = Some(format!("page{}", index + 1));
        }
        let q = CatalogueQuery {
            cursor: (index > 0).then(|| format!("page{index}")),
            expected_catalogue_revision: (index > 0).then(|| identity.catalogue_revision.clone()),
            ..query()
        };
        assert!(matches!(
            checked_result(&q, Ok(page.clone())),
            ControlResponse::EngineParametersV1 {
                result: CatalogueResult::Ready { .. },
                ..
            }
        ));
        accept(&mut cache, &q, page, &engine);
    }
    assert!(cache.ready.is_empty());
    assert!(cache.partial.is_none());
    accept(&mut cache, &query(), page(), &engine);
    assert_eq!(cache.ready.len(), 1);
    cache.observe(
        &query(),
        &error(ControlErrorCode::StaleGeneration, "runtime changed"),
        &engine,
        Some(1),
    );
    assert!(cache.ready.is_empty());
}

fn large_page(start: usize) -> CatalogueResult {
    let mut page = page();
    let CatalogueResult::Ready {
        parameters,
        mappings,
        ..
    } = &mut page
    else {
        panic!()
    };
    let template = parameters[0].clone();
    *parameters = (start..start + 64)
        .map(|i| {
            let mut p = template.clone();
            p.id = format!("parameter{i}");
            p.help = "x".repeat(1024);
            p.side_effects.clear();
            p
        })
        .collect();
    mappings.clear();
    page
}
#[test]
fn encoded_byte_budget_evicts_before_the_entry_limit() {
    let engine = fake_engine();
    let mut cache = CatalogueCache::default();
    for index in 0..20 {
        let mut page = large_page(0);
        let CatalogueResult::Ready { voice_id, .. } = &mut page else {
            panic!()
        };
        *voice_id = Some(format!("voice{index}"));
        let q = CatalogueQuery {
            voice_id: voice_id.clone(),
            ..query()
        };
        accept(&mut cache, &q, page, &engine);
    }
    assert!(!cache.ready.is_empty());
    assert!(cache.ready.len() < 20);
    assert!(cache.ready.len() < MAX_CATALOGUES);
    assert!(cache.bytes <= MAX_CACHED_BYTES);
    assert_eq!(
        cache.bytes,
        cache.ready.iter().map(|e| e.bytes).sum::<usize>()
    );
    assert_eq!(
        cache.ready.back().unwrap().catalogue.voice_id.as_deref(),
        Some("voice19")
    );
}
