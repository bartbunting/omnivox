//! Batch-local logical voice rerouting after runtime synthesis failures.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use omnivox_tts::contracts::{
    AcssApplication, Availability, EngineDescriptor, EngineHealth, FallbackPolicy,
    LogicalVoiceDefinition, NormalizedAcss, PhysicalVoiceId, PostSynthesisApplication,
    PostSynthesisDimension, PostSynthesisStyle, VoiceSelector,
};
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::logical_voices::LogicalVoiceRegistry;
use omnivox_tts::resolver::{resolve_voice, resolve_voice_for_text, VoiceResolution};
use omnivox_tts::routing_policy::RoutingPolicyRegistry;
use omnivox_tts::{
    RequestedAnchor, SynthesisRequest, SynthesisResult, TtsEngine, TtsError, TtsSettings,
};
use tracing::{debug, info, warn};

use crate::health::{EngineAccess, EnginePermit, RuntimeEngineHealth};

/// Maximum synthesis attempts for one routed chunk, including the first try.
pub const MAX_RUNTIME_SYNTHESIS_ATTEMPTS: usize = 4;
const IMPLICIT_LEGACY_VOICE_ID: &str = "__omnivox_internal_legacy__";

/// Immutable registration state plus a batch-local mutable inventory.
#[derive(Clone)]
pub struct LogicalVoiceRoutingSnapshot {
    definitions: Vec<LogicalVoiceDefinition>,
    fallback_policy: FallbackPolicy,
    inventory: Vec<EngineDescriptor>,
    disabled_engine_ids: Vec<String>,
}

impl LogicalVoiceRoutingSnapshot {
    #[cfg(test)]
    pub fn capture(
        logical_voices: &LogicalVoiceRegistry,
        engine_registry: &EngineRegistry,
    ) -> Self {
        Self {
            definitions: logical_voices.definitions().to_vec(),
            fallback_policy: logical_voices.fallback_policy().clone(),
            inventory: engine_registry.inventory(),
            disabled_engine_ids: Vec::new(),
        }
    }

    pub fn capture_with_policy(
        logical_voices: &LogicalVoiceRegistry,
        engine_registry: &EngineRegistry,
        routing_policy: &RoutingPolicyRegistry,
    ) -> Self {
        Self {
            definitions: logical_voices.definitions().to_vec(),
            fallback_policy: routing_policy
                .effective_fallback_policy(logical_voices.fallback_policy()),
            inventory: routing_policy.project_inventory(engine_registry.inventory()),
            disabled_engine_ids: routing_policy.policy().disabled_engine_ids.clone(),
        }
    }

    /// Preview retains its private empty fallback policy but honors explicit
    /// administrative engine disablement.
    pub fn capture_preview(
        logical_voices: &LogicalVoiceRegistry,
        engine_registry: &EngineRegistry,
        routing_policy: &RoutingPolicyRegistry,
    ) -> Self {
        Self {
            definitions: logical_voices.definitions().to_vec(),
            fallback_policy: logical_voices.fallback_policy().clone(),
            inventory: routing_policy.project_inventory(engine_registry.inventory()),
            disabled_engine_ids: routing_policy.policy().disabled_engine_ids.clone(),
        }
    }

    /// Replace the dispatch-time inventory with the worker's current runtime
    /// view before resolving any logical voice in this batch.
    pub fn replace_inventory(&mut self, inventory: Vec<EngineDescriptor>) {
        self.inventory = inventory
            .into_iter()
            .map(|mut descriptor| {
                if self.disabled_engine_ids.contains(&descriptor.id) {
                    descriptor.availability = Availability::Unavailable {
                        reason: "disabled by runtime routing policy".to_owned(),
                    };
                }
                descriptor
            })
            .collect();
    }

    /// Estimate the owned heap payload retained while this routing snapshot is
    /// waiting in the synthesis queue. The work queue also applies an item
    /// limit, so fixed-size request fields do not need separate accounting.
    pub fn queued_payload_bytes(&self) -> usize {
        let definitions = self
            .definitions
            .iter()
            .map(logical_voice_definition_payload_bytes)
            .fold(
                self.definitions
                    .len()
                    .saturating_mul(std::mem::size_of::<LogicalVoiceDefinition>()),
                usize::saturating_add,
            );
        let inventory = self
            .inventory
            .iter()
            .map(engine_descriptor_payload_bytes)
            .fold(
                self.inventory
                    .len()
                    .saturating_mul(std::mem::size_of::<EngineDescriptor>()),
                usize::saturating_add,
            );
        definitions
            .saturating_add(fallback_policy_payload_bytes(&self.fallback_policy))
            .saturating_add(inventory)
            .saturating_add(string_vec_payload_bytes(&self.disabled_engine_ids))
    }

    /// Return the first currently usable engine in global preferred/fallback
    /// order, or DEFAULT when none of those engines is usable.
    pub fn preferred_legacy_engine(
        &self,
        engine_registry: &EngineRegistry,
        default: &Arc<dyn TtsEngine>,
    ) -> Arc<dyn TtsEngine> {
        self.fallback_policy
            .preferred_engines
            .iter()
            .chain(self.fallback_policy.fallback_engines.iter())
            .find_map(|engine_id| {
                self.inventory
                    .iter()
                    .find(|descriptor| descriptor.id == *engine_id)
                    .filter(|descriptor| descriptor.can_synthesize())
                    .and_then(|_| engine_registry.engine(engine_id))
            })
            .unwrap_or_else(|| Arc::clone(default))
    }

    pub fn initial_route(
        &self,
        logical_voice_id: &str,
        engine_registry: &EngineRegistry,
    ) -> Result<LogicalRoute, String> {
        self.resolve_current(logical_voice_id, engine_registry)
    }

    /// Route engine-local legacy state through the normal runtime fallback path.
    pub fn initial_legacy_route(
        &mut self,
        requested_voice: PhysicalVoiceId,
        engine_registry: &EngineRegistry,
    ) -> Result<LogicalRoute, String> {
        self.definitions
            .retain(|definition| definition.id != IMPLICIT_LEGACY_VOICE_ID);
        self.definitions.push(LogicalVoiceDefinition {
            id: IMPLICIT_LEGACY_VOICE_ID.to_owned(),
            language: None,
            preferences: vec![VoiceSelector::Exact(requested_voice)],
            acss: NormalizedAcss::default(),
            effects: PostSynthesisStyle::default(),
        });
        let mut route = self.resolve_current(IMPLICIT_LEGACY_VOICE_ID, engine_registry)?;
        route.reported_logical_voice_id = None;
        Ok(route)
    }

    /// Exclude the failed runtime target locally and resolve the same logical
    /// voice again. Invalid settings are not route failures and are not retried.
    pub fn reroute_after_failure(
        &mut self,
        route: &LogicalRoute,
        error: &TtsError,
        engine_registry: &EngineRegistry,
    ) -> RuntimeReroute {
        if !record_runtime_failure(&mut self.inventory, &route.realized, error) {
            return RuntimeReroute::NotRetryable;
        }
        match self.resolve_current(&route.logical_voice_id, engine_registry) {
            Ok(mut retry) if retry.realized != route.realized => {
                retry.reported_logical_voice_id = route.reported_logical_voice_id.clone();
                RuntimeReroute::Retry(Box::new(retry))
            }
            Ok(_) => RuntimeReroute::Exhausted(format!(
                "logical voice {} resolved back to its failed runtime target",
                route.logical_voice_id
            )),
            Err(error) => RuntimeReroute::Exhausted(error),
        }
    }

    /// Resolve a route whose documented input repertoire can preserve TEXT.
    ///
    /// This is deliberately batch-local and does not mark an otherwise healthy
    /// engine failed.  A later compatible chunk can therefore return to the
    /// preferred route.
    fn route_for_text(
        &self,
        route: &LogicalRoute,
        text: &str,
        engine_registry: &EngineRegistry,
    ) -> Result<LogicalRoute, String> {
        let current_incompatibility = self.text_incompatibility(route, text);

        let mut resolved = self
            .resolve_current_with_text(&route.logical_voice_id, Some(text), engine_registry)
            .map_err(|error| match current_incompatibility {
                Some((utf8_offset, character)) => format!(
                    "{error}; current engine {} cannot encode U+{:04X} at UTF-8 byte offset {}",
                    route.realized.engine_id,
                    u32::from(character),
                    utf8_offset
                ),
                None => error,
            })?;
        resolved.reported_logical_voice_id = route.reported_logical_voice_id.clone();
        Ok(resolved)
    }

    fn text_incompatibility(&self, route: &LogicalRoute, text: &str) -> Option<(usize, char)> {
        self.inventory
            .iter()
            .find(|descriptor| descriptor.id == route.realized.engine_id)
            .and_then(|descriptor| {
                descriptor
                    .capabilities
                    .text_repertoire
                    .first_unsupported(text)
            })
    }

    fn resolve_current(
        &self,
        logical_voice_id: &str,
        engine_registry: &EngineRegistry,
    ) -> Result<LogicalRoute, String> {
        self.resolve_current_with_text(logical_voice_id, None, engine_registry)
    }

    fn resolve_current_with_text(
        &self,
        logical_voice_id: &str,
        text: Option<&str>,
        engine_registry: &EngineRegistry,
    ) -> Result<LogicalRoute, String> {
        let definition = self
            .definitions
            .iter()
            .find(|definition| definition.id == logical_voice_id)
            .ok_or_else(|| {
                format!("logical voice {logical_voice_id} no longer has a definition")
            })?;
        let resolution = match text {
            Some(text) => {
                resolve_voice_for_text(&self.inventory, definition, &self.fallback_policy, text)
            }
            None => resolve_voice(&self.inventory, definition, &self.fallback_policy),
        }
        .map_err(|error| error.to_string())?;
        route_from_resolution(resolution, definition, &self.inventory, engine_registry)
    }
}

fn logical_voice_definition_payload_bytes(definition: &LogicalVoiceDefinition) -> usize {
    definition
        .id
        .len()
        .saturating_add(definition.language.as_ref().map_or(0, String::len))
        .saturating_add(
            definition
                .preferences
                .len()
                .saturating_mul(std::mem::size_of::<VoiceSelector>()),
        )
        .saturating_add(
            definition
                .preferences
                .iter()
                .map(voice_selector_payload_bytes)
                .fold(0usize, usize::saturating_add),
        )
}

fn fallback_policy_payload_bytes(policy: &FallbackPolicy) -> usize {
    string_vec_payload_bytes(&policy.preferred_engines)
        .saturating_add(string_vec_payload_bytes(&policy.fallback_engines))
        .saturating_add(
            policy
                .global_default
                .as_ref()
                .map_or(0, voice_selector_payload_bytes),
        )
}

fn voice_selector_payload_bytes(selector: &VoiceSelector) -> usize {
    match selector {
        VoiceSelector::Exact(id) => id.engine_id.len().saturating_add(id.voice_id.len()),
        VoiceSelector::EngineDefault { engine_id } => engine_id.len(),
        VoiceSelector::Properties {
            engine_id,
            language,
            ..
        } => engine_id
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(language.as_ref().map_or(0, String::len)),
    }
}

fn engine_descriptor_payload_bytes(descriptor: &EngineDescriptor) -> usize {
    let voices = descriptor
        .voices
        .iter()
        .map(|voice| {
            voice
                .id
                .engine_id
                .len()
                .saturating_add(voice.id.voice_id.len())
                .saturating_add(voice.display_name.len())
                .saturating_add(voice.language.as_ref().map_or(0, String::len))
                .saturating_add(availability_payload_bytes(&voice.availability))
        })
        .fold(
            std::mem::size_of_val(descriptor.voices.as_slice()),
            usize::saturating_add,
        );
    descriptor
        .id
        .len()
        .saturating_add(descriptor.display_name.len())
        .saturating_add(descriptor.version.as_ref().map_or(0, String::len))
        .saturating_add(availability_payload_bytes(&descriptor.availability))
        .saturating_add(engine_health_payload_bytes(&descriptor.health))
        .saturating_add(
            descriptor
                .capabilities
                .post_synthesis_dimensions
                .len()
                .saturating_mul(std::mem::size_of::<PostSynthesisDimension>()),
        )
        .saturating_add(
            descriptor
                .capabilities
                .native_extensions
                .iter()
                .map(|extension| extension.id.len().saturating_add(extension.description.len()))
                .fold(
                    std::mem::size_of_val(
                        descriptor.capabilities.native_extensions.as_slice(),
                    ),
                    usize::saturating_add,
                ),
        )
        .saturating_add(voices)
        .saturating_add(descriptor.default_voice_id.as_ref().map_or(0, String::len))
}

fn availability_payload_bytes(availability: &Availability) -> usize {
    match availability {
        Availability::Available => 0,
        Availability::Unavailable { reason } => reason.len(),
    }
}

fn engine_health_payload_bytes(health: &EngineHealth) -> usize {
    match health {
        EngineHealth::Healthy => 0,
        EngineHealth::Degraded { reason } | EngineHealth::Failed { reason } => reason.len(),
    }
}

fn string_vec_payload_bytes(values: &[String]) -> usize {
    values
        .iter()
        .map(String::len)
        .fold(
            values
                .len()
                .saturating_mul(std::mem::size_of::<String>()),
            usize::saturating_add,
        )
}

/// Resolve legacy, engine-neutral voice state against the selected engine.
///
/// Legacy protocol state carries one unscoped selector, so changing the
/// preferred engine can leave a WinRT language or voice ID in front of a
/// helper which only accepts its own native IDs.  Preserve selectors the
/// selected engine advertises, canonicalize name/language matches to a
/// physical ID, and otherwise degrade to that engine's advertised default.
pub(crate) fn legacy_voice_for_engine(engine: &dyn TtsEngine, requested: &str) -> String {
    let descriptor = engine.descriptor();
    let engine_prefix = format!("{}:", descriptor.id);
    let requested_without_prefix = requested.strip_prefix(&engine_prefix);
    let exact = descriptor
        .voices
        .iter()
        .filter(|voice| matches!(&voice.availability, Availability::Available))
        .find(|voice| {
            voice.id.voice_id == requested
                || voice
                    .id
                    .voice_id
                    .strip_prefix(&engine_prefix)
                    .is_some_and(|voice_id| voice_id == requested)
                || requested_without_prefix
                    .is_some_and(|requested| requested == voice.id.voice_id)
        });
    let named = exact.or_else(|| {
        descriptor
            .voices
            .iter()
            .filter(|voice| matches!(&voice.availability, Availability::Available))
            .find(|voice| {
                voice.display_name == requested
                    || voice
                        .language
                        .as_deref()
                        .is_some_and(|language| language.eq_ignore_ascii_case(requested))
            })
    });
    let selected = named
        .map(|voice| voice.id.voice_id.clone())
        .or_else(|| descriptor.default_voice_id.filter(|voice| !voice.is_empty()))
        .unwrap_or_else(|| requested.to_owned());
    if selected != requested {
        debug!(
            "Legacy voice selector {:?} resolved to engine {} voice {:?}",
            requested, descriptor.id, selected
        );
    }
    selected
}

/// Physical route selected for one logical voice within a dispatched batch.
pub struct LogicalRoute {
    pub logical_voice_id: String,
    pub reported_logical_voice_id: Option<String>,
    pub engine: Arc<dyn TtsEngine>,
    pub realized: PhysicalVoiceId,
    pub acss: AcssApplication,
    pub effects: PostSynthesisApplication,
}

pub enum RuntimeReroute {
    Retry(Box<LogicalRoute>),
    NotRetryable,
    Exhausted(String),
}

pub enum RuntimeSynthesisOutcome {
    Ready(Box<SynthesisResult>),
    Cancelled,
    Failed,
    Exhausted,
}

/// Synthesize one chunk, re-resolving the logical voice after retryable
/// failures. Every attempt speaks the identical text and checks cancellation
/// both before and after the synchronous engine call.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn synthesize_with_runtime_fallback(
    chunk: &str,
    settings: &TtsSettings,
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    generation: u64,
    generation_counter: &AtomicU64,
) -> RuntimeSynthesisOutcome {
    synthesize_with_runtime_fallback_anchored(
        chunk,
        &[],
        settings,
        route,
        routing,
        engine_registry,
        runtime_health,
        generation,
        generation_counter,
    )
}

/// Routed synthesis retaining identical requested anchors across every retry.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_with_runtime_fallback_anchored(
    chunk: &str,
    anchors: &[RequestedAnchor],
    settings: &TtsSettings,
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    generation: u64,
    generation_counter: &AtomicU64,
) -> RuntimeSynthesisOutcome {
    synthesize_with_runtime_fallback_anchored_inner(
        chunk,
        anchors,
        settings,
        None,
        route,
        routing,
        engine_registry,
        runtime_health,
        generation,
        generation_counter,
    )
}

/// Routed synthesis with a complete per-span ACSS style reapplied after every
/// engine fallback.
#[allow(clippy::too_many_arguments)]
pub fn synthesize_with_runtime_fallback_anchored_styled(
    chunk: &str,
    anchors: &[RequestedAnchor],
    settings: &TtsSettings,
    requested_acss: &NormalizedAcss,
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    generation: u64,
    generation_counter: &AtomicU64,
) -> RuntimeSynthesisOutcome {
    synthesize_with_runtime_fallback_anchored_inner(
        chunk,
        anchors,
        settings,
        Some(requested_acss),
        route,
        routing,
        engine_registry,
        runtime_health,
        generation,
        generation_counter,
    )
}

#[allow(clippy::too_many_arguments)]
fn synthesize_with_runtime_fallback_anchored_inner(
    chunk: &str,
    anchors: &[RequestedAnchor],
    settings: &TtsSettings,
    requested_acss: Option<&NormalizedAcss>,
    route: &mut LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    runtime_health: &RuntimeEngineHealth,
    generation: u64,
    generation_counter: &AtomicU64,
) -> RuntimeSynthesisOutcome {
    if stale(generation, generation_counter) {
        return RuntimeSynthesisOutcome::Cancelled;
    }
    let previous_incompatibility = routing.text_incompatibility(route, chunk);
    let compatible_route = match routing.route_for_text(route, chunk, engine_registry) {
        Ok(compatible_route) => compatible_route,
        Err(error) => {
            warn!(
                "Logical voice {} has no route capable of preserving this chunk: {}",
                route.logical_voice_id, error
            );
            return RuntimeSynthesisOutcome::Exhausted;
        }
    };
    if compatible_route.realized != route.realized {
        if let Some((utf8_offset, character)) = previous_incompatibility {
            info!(
                logical_voice = route.logical_voice_id,
                previous_engine_id = route.realized.engine_id,
                previous_voice_id = route.realized.voice_id,
                engine_id = compatible_route.realized.engine_id,
                voice_id = compatible_route.realized.voice_id,
                unsupported_codepoint = format_args!("U+{:04X}", u32::from(character)),
                utf8_offset,
                "Rerouted chunk to preserve source text"
            );
        } else {
            debug!(
                logical_voice = route.logical_voice_id,
                previous_engine_id = route.realized.engine_id,
                previous_voice_id = route.realized.voice_id,
                engine_id = compatible_route.realized.engine_id,
                voice_id = compatible_route.realized.voice_id,
                "Restored preferred route for compatible text"
            );
        }
    }
    *route = compatible_route;

    for attempt in 1..=MAX_RUNTIME_SYNTHESIS_ATTEMPTS {
        if stale(generation, generation_counter) {
            return RuntimeSynthesisOutcome::Cancelled;
        }
        let permit = match runtime_health.acquire(&route.realized.engine_id) {
            EngineAccess::Permit(permit) => permit,
            EngineAccess::Denied { reason } => {
                warn!(
                    "Logical voice {} routed around engine {}: {}",
                    route.logical_voice_id, route.realized.engine_id, reason
                );
                let error = TtsError::NotAvailable;
                match select_retry(route, routing, engine_registry, &error, attempt) {
                    Ok(retry) => {
                        *route = retry;
                        continue;
                    }
                    Err(outcome) => return outcome,
                }
            }
        };
        if stale(generation, generation_counter) {
            release_probe_if_held(runtime_health, &route.realized.engine_id, permit);
            return RuntimeSynthesisOutcome::Cancelled;
        }
        if permit == EnginePermit::RecoveryProbe {
            if let Err(preparation_error) = route.engine.prepare_recovery_probe() {
                if stale(generation, generation_counter) {
                    release_probe_if_held(runtime_health, &route.realized.engine_id, permit);
                    return RuntimeSynthesisOutcome::Cancelled;
                }
                let error = TtsError::SynthesisFailed(format!(
                    "recovery preparation failed: {preparation_error}"
                ));
                let cooldown =
                    runtime_health.record_failure(&route.realized.engine_id, error.to_string());
                warn!(
                    "Engine {} recovery preparation failed; circuit opened for {} seconds: {}",
                    route.realized.engine_id,
                    cooldown.as_secs(),
                    preparation_error
                );
                match select_retry(route, routing, engine_registry, &error, attempt) {
                    Ok(retry) => {
                        *route = retry;
                        continue;
                    }
                    Err(outcome) => return outcome,
                }
            }
        }
        let mut routed_settings = settings.clone();
        routed_settings.voice = route.realized.voice_id.clone();
        let acss = requested_acss.map_or_else(
            || route.acss.clone(),
            |style| {
                style.clone().degrade_for(
                    &route.engine.descriptor().capabilities.acss,
                )
            },
        );
        apply_normalized_acss(&mut routed_settings, &acss.style);
        let mut request = SynthesisRequest::new(chunk, routed_settings)
            .with_normalized_acss(acss.style.clone());
        request.requested_voice = Some(route.realized.clone());
        request.logical_voice_id = route.reported_logical_voice_id.clone();
        request.anchors = anchors.to_vec();
        let started_at = Instant::now();
        info!(
            logical_voice = route.logical_voice_id,
            engine_id = route.realized.engine_id,
            voice_id = route.realized.voice_id,
            attempt,
            text_bytes = chunk.len(),
            recovery_probe = permit == EnginePermit::RecoveryProbe,
            "Starting routed synthesis"
        );
        if crate::diagnostics::synthesis_text_logging_enabled() {
            info!(
                logical_voice = route.logical_voice_id,
                engine_id = route.realized.engine_id,
                voice_id = route.realized.voice_id,
                attempt,
                generation,
                synthesis_text = ?chunk,
                "Captured synthesis text"
            );
        }
        let synthesis = route.engine.synthesize(&request).and_then(|mut result| {
            result.resolve_anchors(
                &request,
                route
                    .engine
                    .descriptor()
                    .capabilities
                    .markers
                    .requested_anchors,
            );
            result.validate(&request)?;
            result.degraded_acss = acss.omitted.clone();
            Ok(result)
        });
        match synthesis {
            Ok(result) => {
                runtime_health.record_success(&route.realized.engine_id, permit);
                info!(
                    logical_voice = route.logical_voice_id,
                    engine_id = route.realized.engine_id,
                    voice_id = route.realized.voice_id,
                    attempt,
                    frames = result.audio.frame_count(),
                    elapsed_ms = started_at.elapsed().as_millis(),
                    recovered = permit == EnginePermit::RecoveryProbe,
                    "Routed synthesis completed"
                );
                return if stale(generation, generation_counter) {
                    RuntimeSynthesisOutcome::Cancelled
                } else {
                    RuntimeSynthesisOutcome::Ready(Box::new(result))
                };
            }
            Err(error) => {
                if stale(generation, generation_counter) {
                    release_probe_if_held(runtime_health, &route.realized.engine_id, permit);
                    return RuntimeSynthesisOutcome::Cancelled;
                }
                match (&error, permit) {
                    (TtsError::SynthesisFailed(_), _)
                    | (TtsError::NotAvailable, EnginePermit::RecoveryProbe) => {
                        let cooldown = runtime_health
                            .record_failure(&route.realized.engine_id, error.to_string());
                        warn!(
                            "Engine {} runtime circuit opened for {} seconds",
                            route.realized.engine_id,
                            cooldown.as_secs()
                        );
                    }
                    (TtsError::NotAvailable, EnginePermit::Normal)
                    | (TtsError::VoiceNotFound(_), _)
                    | (TtsError::InvalidParameter(_), _) => {
                        release_probe_if_held(runtime_health, &route.realized.engine_id, permit);
                    }
                }
                warn!(
                    "Logical voice {} synthesis attempt {}/{} failed on engine {} voice {}: {}",
                    route.logical_voice_id,
                    attempt,
                    MAX_RUNTIME_SYNTHESIS_ATTEMPTS,
                    route.realized.engine_id,
                    route.realized.voice_id,
                    error
                );
                match select_retry(route, routing, engine_registry, &error, attempt) {
                    Ok(retry) => *route = retry,
                    Err(outcome) => return outcome,
                }
            }
        }
    }

    unreachable!("the bounded routed synthesis loop always returns")
}

fn select_retry(
    route: &LogicalRoute,
    routing: &mut LogicalVoiceRoutingSnapshot,
    engine_registry: &EngineRegistry,
    error: &TtsError,
    attempt: usize,
) -> Result<LogicalRoute, RuntimeSynthesisOutcome> {
    let reroute = routing.reroute_after_failure(route, error, engine_registry);
    if attempt == MAX_RUNTIME_SYNTHESIS_ATTEMPTS {
        warn!(
            "Logical voice {} exhausted the runtime synthesis attempt limit",
            route.logical_voice_id
        );
        return Err(RuntimeSynthesisOutcome::Exhausted);
    }
    match reroute {
        RuntimeReroute::Retry(retry) => {
            info!(
                "Logical voice {} retrying on engine {} voice {}",
                retry.logical_voice_id, retry.realized.engine_id, retry.realized.voice_id
            );
            Ok(*retry)
        }
        RuntimeReroute::NotRetryable => Err(RuntimeSynthesisOutcome::Failed),
        RuntimeReroute::Exhausted(reason) => {
            warn!(
                "Logical voice {} runtime fallback exhausted: {}",
                route.logical_voice_id, reason
            );
            Err(RuntimeSynthesisOutcome::Exhausted)
        }
    }
}

fn release_probe_if_held(
    runtime_health: &RuntimeEngineHealth,
    engine_id: &str,
    permit: EnginePermit,
) {
    if permit == EnginePermit::RecoveryProbe {
        runtime_health.release_probe(engine_id);
    }
}

fn stale(generation: u64, generation_counter: &AtomicU64) -> bool {
    generation_counter.load(Ordering::Acquire) != generation
}

fn route_from_resolution(
    resolution: VoiceResolution,
    definition: &LogicalVoiceDefinition,
    inventory: &[EngineDescriptor],
    engine_registry: &EngineRegistry,
) -> Result<LogicalRoute, String> {
    let engine = engine_registry
        .engine(&resolution.realized.engine_id)
        .ok_or_else(|| {
            format!(
                "logical voice {} resolved to missing engine {}",
                resolution.logical_voice_id, resolution.realized.engine_id
            )
        })?;
    let descriptor = inventory
        .iter()
        .find(|descriptor| descriptor.id == resolution.realized.engine_id)
        .ok_or_else(|| {
            format!(
                "logical voice {} resolved to an engine missing from inventory",
                resolution.logical_voice_id
            )
        })?;
    let acss = definition
        .acss
        .clone()
        .degrade_for(&descriptor.capabilities.acss);
    if !acss.omitted.is_empty() {
        debug!(
            "Logical voice {} omitted unsupported {:?} on engine {}",
            resolution.logical_voice_id, acss.omitted, resolution.realized.engine_id
        );
    }
    let effects = definition
        .effects
        .clone()
        .degrade_for(&descriptor.capabilities.post_synthesis_dimensions);
    if !effects.omitted.is_empty() {
        debug!(
            "Logical voice {} omitted unsupported post-synthesis {:?} on engine {}",
            resolution.logical_voice_id, effects.omitted, resolution.realized.engine_id
        );
    }

    Ok(LogicalRoute {
        reported_logical_voice_id: Some(resolution.logical_voice_id.clone()),
        logical_voice_id: resolution.logical_voice_id,
        engine,
        realized: resolution.realized,
        acss,
        effects,
    })
}

pub(crate) fn apply_normalized_acss(settings: &mut TtsSettings, style: &NormalizedAcss) {
    if let Some(rate) = style.rate {
        settings.rate = rate;
    }
    if let Some(average_pitch) = style.average_pitch {
        settings.pitch = normalized_average_pitch(average_pitch);
    }
    if let Some(volume) = style.volume {
        settings.volume = volume;
    }
}

/// Interpolate the ten ACSS pitch levels used by the Emacsvox adapter.
fn normalized_average_pitch(value: f32) -> f32 {
    const PITCH_LEVELS: [f32; 10] = [0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.2, 1.4, 1.7, 2.0];
    let position = value.clamp(0.0, 1.0) * (PITCH_LEVELS.len() - 1) as f32;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(PITCH_LEVELS.len() - 1);
    let fraction = position - lower as f32;
    PITCH_LEVELS[lower] + (PITCH_LEVELS[upper] - PITCH_LEVELS[lower]) * fraction
}

fn record_runtime_failure(
    inventory: &mut [EngineDescriptor],
    realized: &PhysicalVoiceId,
    error: &TtsError,
) -> bool {
    let Some(engine) = inventory
        .iter_mut()
        .find(|engine| engine.id == realized.engine_id)
    else {
        return false;
    };

    match error {
        TtsError::VoiceNotFound(reason) => {
            let Some(voice) = engine.voices.iter_mut().find(|voice| voice.id == *realized) else {
                return false;
            };
            voice.availability = Availability::Unavailable {
                reason: reason.clone(),
            };
            true
        }
        TtsError::NotAvailable => {
            engine.availability = Availability::Unavailable {
                reason: "runtime synthesis reported engine unavailable".to_owned(),
            };
            true
        }
        TtsError::SynthesisFailed(reason) => {
            engine.health = EngineHealth::Failed {
                reason: reason.clone(),
            };
            true
        }
        TtsError::InvalidParameter(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    use super::*;
    use omnivox_tts::contracts::{
        AcssCapabilities, AudioOutputMode, CancellationSupport, ConcurrencyModel,
        EngineCapabilities, MarkerCapabilities, NormalizedAcss, PostSynthesisDimension,
        PostSynthesisStyle, TextRepertoire, VoiceDescriptor, VoiceSelector,
    };
    use omnivox_tts::routing_policy::RoutingPolicy;
    use omnivox_tts::{
        AudioBuffer, SynthesisRequest, SynthesisResult, TtsSettings, VoiceInfo, VoiceQuality,
    };

    enum MockFailure {
        NotAvailableOnce(AtomicUsize),
        Synthesis,
        SynthesisOnce(AtomicUsize),
        SynthesisAndCancel(Arc<AtomicU64>),
    }

    struct MockEngine {
        descriptor: EngineDescriptor,
        failure: Option<MockFailure>,
        recovery_preparations: AtomicUsize,
        calls: Mutex<Vec<(String, String)>>,
        anchors: Mutex<Vec<Vec<RequestedAnchor>>>,
        settings: Mutex<Vec<TtsSettings>>,
        normalized_acss: Mutex<Vec<NormalizedAcss>>,
        logical_voice_ids: Mutex<Vec<Option<String>>>,
    }

    impl TtsEngine for MockEngine {
        fn descriptor(&self) -> EngineDescriptor {
            self.descriptor.clone()
        }

        fn prepare_recovery_probe(&self) -> Result<(), TtsError> {
            self.recovery_preparations.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
            self.calls
                .lock()
                .unwrap()
                .push((request.text.clone(), request.settings.voice.clone()));
            self.anchors.lock().unwrap().push(request.anchors.clone());
            self.settings.lock().unwrap().push(request.settings.clone());
            self.normalized_acss
                .lock()
                .unwrap()
                .push(request.normalized_acss.clone());
            self.logical_voice_ids
                .lock()
                .unwrap()
                .push(request.logical_voice_id.clone());
            let success = || {
                Ok(SynthesisResult::audio(
                    self.descriptor.id.clone(),
                    request.requested_voice.clone(),
                    AudioBuffer::empty(),
                ))
            };
            match self.failure.as_ref() {
                Some(MockFailure::NotAvailableOnce(remaining))
                    if remaining.swap(0, Ordering::AcqRel) > 0 =>
                {
                    Err(TtsError::NotAvailable)
                }
                Some(MockFailure::Synthesis) => {
                    Err(TtsError::SynthesisFailed("mock failure".to_owned()))
                }
                Some(MockFailure::SynthesisOnce(remaining))
                    if remaining.swap(0, Ordering::AcqRel) > 0 =>
                {
                    Err(TtsError::SynthesisFailed("one mock failure".to_owned()))
                }
                Some(MockFailure::SynthesisAndCancel(counter)) => {
                    counter.fetch_add(1, Ordering::Release);
                    Err(TtsError::SynthesisFailed("cancelled mock".to_owned()))
                }
                Some(MockFailure::NotAvailableOnce(_) | MockFailure::SynthesisOnce(_)) => {
                    success()
                }
                None => success(),
            }
        }

        fn stop(&self) {}

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

    fn descriptor(engine_id: &str, voice_ids: &[&str]) -> EngineDescriptor {
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
                text_repertoire: omnivox_tts::contracts::TextRepertoire::Unicode,
                post_synthesis_dimensions:
                    omnivox_tts::contracts::buffered_post_synthesis_dimensions(),
                native_extensions: Vec::new(),
            },
            voices: voice_ids
                .iter()
                .map(|voice_id| VoiceDescriptor {
                    id: PhysicalVoiceId::new(engine_id, *voice_id),
                    display_name: (*voice_id).to_owned(),
                    language: Some("en-US".to_owned()),
                    gender: None,
                    quality: VoiceQuality::Compact,
                    availability: Availability::Available,
                })
                .collect(),
            default_voice_id: voice_ids.first().map(|voice_id| (*voice_id).to_owned()),
        }
    }

    fn register_engine(registry: &mut EngineRegistry, engine_id: &str, voice_ids: &[&str]) {
        registry
            .register(Arc::new(MockEngine {
                descriptor: descriptor(engine_id, voice_ids),
                failure: None,
                recovery_preparations: AtomicUsize::new(0),
                calls: Mutex::new(Vec::new()),
                anchors: Mutex::new(Vec::new()),
                settings: Mutex::new(Vec::new()),
                normalized_acss: Mutex::new(Vec::new()),
                logical_voice_ids: Mutex::new(Vec::new()),
            }))
            .unwrap();
    }

    fn synthesis_engine(
        engine_id: &str,
        voice_id: &str,
        failure: Option<MockFailure>,
    ) -> Arc<MockEngine> {
        Arc::new(MockEngine {
            descriptor: descriptor(engine_id, &[voice_id]),
            failure,
            recovery_preparations: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            anchors: Mutex::new(Vec::new()),
            settings: Mutex::new(Vec::new()),
            normalized_acss: Mutex::new(Vec::new()),
            logical_voice_ids: Mutex::new(Vec::new()),
        })
    }

    fn synthesis_engine_with_acss(
        engine_id: &str,
        voice_id: &str,
        acss: AcssCapabilities,
    ) -> Arc<MockEngine> {
        let mut engine_descriptor = descriptor(engine_id, &[voice_id]);
        engine_descriptor.capabilities.acss = acss;
        Arc::new(MockEngine {
            descriptor: engine_descriptor,
            failure: None,
            recovery_preparations: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            anchors: Mutex::new(Vec::new()),
            settings: Mutex::new(Vec::new()),
            normalized_acss: Mutex::new(Vec::new()),
            logical_voice_ids: Mutex::new(Vec::new()),
        })
    }

    fn synthesis_engine_with_repertoire(
        engine_id: &str,
        voice_id: &str,
        text_repertoire: TextRepertoire,
    ) -> Arc<MockEngine> {
        let mut engine_descriptor = descriptor(engine_id, &[voice_id]);
        engine_descriptor.capabilities.text_repertoire = text_repertoire;
        Arc::new(MockEngine {
            descriptor: engine_descriptor,
            failure: None,
            recovery_preparations: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            anchors: Mutex::new(Vec::new()),
            settings: Mutex::new(Vec::new()),
            normalized_acss: Mutex::new(Vec::new()),
            logical_voice_ids: Mutex::new(Vec::new()),
        })
    }

    fn definition(preferences: Vec<VoiceSelector>) -> LogicalVoiceDefinition {
        LogicalVoiceDefinition {
            id: "source-code".to_owned(),
            language: Some("en-US".to_owned()),
            preferences,
            acss: NormalizedAcss::default(),
            effects: Default::default(),
        }
    }

    fn exact(engine_id: &str, voice_id: &str) -> VoiceSelector {
        VoiceSelector::Exact(PhysicalVoiceId::new(engine_id, voice_id))
    }

    fn snapshot(
        engines: &EngineRegistry,
        definition: LogicalVoiceDefinition,
        fallback_policy: FallbackPolicy,
    ) -> LogicalVoiceRoutingSnapshot {
        let mut logical_voices = LogicalVoiceRegistry::default();
        logical_voices
            .register(1, vec![definition], fallback_policy, &engines.inventory())
            .unwrap();
        LogicalVoiceRoutingSnapshot::capture(&logical_voices, engines)
    }

    #[test]
    fn global_policy_selects_legacy_engine_and_skips_disabled_positions() {
        let winrt = synthesis_engine("winrt", "david", None);
        let eloquence = synthesis_engine("eloquence", "reed", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&winrt) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&eloquence) as Arc<dyn TtsEngine>)
            .unwrap();
        let logical = LogicalVoiceRegistry::default();
        let mut policy = RoutingPolicyRegistry::new("winrt");
        policy
            .register(
                1,
                RoutingPolicy {
                    preferred_engine_ids: vec!["eloquence".to_owned(), "winrt".to_owned()],
                    fallback_engine_ids: Vec::new(),
                    disabled_engine_ids: Vec::new(),
                },
            )
            .unwrap();
        let preferred = LogicalVoiceRoutingSnapshot::capture_with_policy(
            &logical, &engines, &policy,
        );

        assert_eq!(
            preferred
                .preferred_legacy_engine(&engines, &(winrt.clone() as Arc<dyn TtsEngine>))
                .descriptor()
                .id,
            "eloquence"
        );

        policy
            .register(
                2,
                RoutingPolicy {
                    preferred_engine_ids: vec!["eloquence".to_owned(), "winrt".to_owned()],
                    fallback_engine_ids: Vec::new(),
                    disabled_engine_ids: vec!["eloquence".to_owned()],
                },
            )
            .unwrap();
        let disabled = LogicalVoiceRoutingSnapshot::capture_with_policy(
            &logical, &engines, &policy,
        );
        assert_eq!(
            disabled
                .preferred_legacy_engine(&engines, &(winrt.clone() as Arc<dyn TtsEngine>))
                .descriptor()
                .id,
            "winrt"
        );
    }

    #[test]
    fn legacy_voice_state_degrades_to_each_selected_engines_native_default() {
        let dectalk = synthesis_engine("dectalk", "paul", None);
        let eloquence = synthesis_engine("eloquence", "v1", None);

        assert_eq!(legacy_voice_for_engine(&*dectalk, "en-US"), "paul");
        assert_eq!(legacy_voice_for_engine(&*eloquence, "en-US"), "v1");
        assert_eq!(legacy_voice_for_engine(&*dectalk, "paul"), "paul");
        assert_eq!(legacy_voice_for_engine(&*eloquence, "paul"), "v1");
    }

    #[test]
    fn routed_synthesis_applies_only_supported_logical_acss() {
        let engine = synthesis_engine_with_acss(
            "reference",
            "voice",
            AcssCapabilities {
                rate: true,
                average_pitch: true,
                pitch_range: true,
                stress: true,
                richness: true,
                ..AcssCapabilities::default()
            },
        );
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&engine) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut logical_definition = definition(vec![exact("reference", "voice")]);
        logical_definition.acss = NormalizedAcss {
            rate: Some(0.8),
            average_pitch: Some(5.0 / 9.0),
            pitch_range: Some(0.3),
            stress: Some(0.4),
            richness: Some(0.7),
            volume: Some(0.2),
            ..NormalizedAcss::default()
        };
        let mut routes = snapshot(&engines, logical_definition, FallbackPolicy::default());
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(1);
        let base = TtsSettings {
            voice: "legacy".to_owned(),
            rate: 0.3,
            pitch: 1.7,
            volume: 0.9,
        };

        let outcome = synthesize_with_runtime_fallback(
            "styled",
            &base,
            &mut route,
            &mut routes,
            &engines,
            &health,
            1,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(
            route.acss.omitted,
            [omnivox_tts::contracts::AcssDimension::Volume]
        );
        let settings = engine.settings.lock().unwrap();
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].voice, "voice");
        assert!((settings[0].rate - 0.8).abs() < f32::EPSILON);
        assert!((settings[0].pitch - 1.0).abs() < f32::EPSILON);
        assert!((settings[0].volume - 0.9).abs() < f32::EPSILON);
        assert_eq!(
            engine.normalized_acss.lock().unwrap()[0],
            NormalizedAcss {
                rate: Some(0.8),
                average_pitch: Some(5.0 / 9.0),
                pitch_range: Some(0.3),
                stress: Some(0.4),
                richness: Some(0.7),
                volume: None,
            }
        );
    }

    #[test]
    fn average_pitch_mapping_matches_emacsvox_acss_levels() {
        let expected = [0.5_f32, 0.6, 0.7, 0.8, 0.9, 1.0, 1.2, 1.4, 1.7, 2.0];
        for (index, pitch) in expected.into_iter().enumerate() {
            let normalized = index as f32 / 9.0;
            assert!((normalized_average_pitch(normalized) - pitch).abs() < 0.000_001);
        }
    }

    #[test]
    fn fallback_recomputes_acss_for_the_new_engine() {
        let primary = synthesis_engine_with_acss(
            "rate-only",
            "primary",
            AcssCapabilities {
                rate: true,
                ..AcssCapabilities::default()
            },
        );
        let fallback = synthesis_engine_with_acss(
            "pitch-only",
            "fallback",
            AcssCapabilities {
                average_pitch: true,
                ..AcssCapabilities::default()
            },
        );
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut logical_definition = definition(vec![
            exact("rate-only", "primary"),
            exact("pitch-only", "fallback"),
        ]);
        logical_definition.acss = NormalizedAcss {
            rate: Some(0.7),
            average_pitch: Some(0.4),
            ..NormalizedAcss::default()
        };
        let mut routes = snapshot(&engines, logical_definition, FallbackPolicy::default());
        let initial = routes.initial_route("source-code", &engines).unwrap();
        assert_eq!(initial.acss.style.rate, Some(0.7));
        assert_eq!(initial.acss.style.average_pitch, None);

        let RuntimeReroute::Retry(retry) = routes.reroute_after_failure(
            &initial,
            &TtsError::VoiceNotFound("primary disappeared".to_owned()),
            &engines,
        ) else {
            panic!("voice failure did not select the alternate engine");
        };
        assert_eq!(retry.realized.engine_id, "pitch-only");
        assert_eq!(retry.acss.style.rate, None);
        assert_eq!(retry.acss.style.average_pitch, Some(0.4));
    }

    #[test]
    fn fallback_recomputes_post_synthesis_effects_for_the_new_engine() {
        let mut primary = synthesis_engine("gain-only", "primary", None);
        Arc::get_mut(&mut primary)
            .unwrap()
            .descriptor
            .capabilities
            .post_synthesis_dimensions = vec![PostSynthesisDimension::Gain];
        let mut fallback = synthesis_engine("pan-only", "fallback", None);
        Arc::get_mut(&mut fallback)
            .unwrap()
            .descriptor
            .capabilities
            .post_synthesis_dimensions = vec![PostSynthesisDimension::Pan];
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut logical_definition = definition(vec![
            exact("gain-only", "primary"),
            exact("pan-only", "fallback"),
        ]);
        logical_definition.effects = PostSynthesisStyle {
            gain: Some(0.7),
            pan: Some(0.2),
            ..PostSynthesisStyle::default()
        };
        let mut routes = snapshot(&engines, logical_definition, FallbackPolicy::default());
        let initial = routes.initial_route("source-code", &engines).unwrap();
        assert_eq!(initial.effects.style.gain, Some(0.7));
        assert_eq!(initial.effects.style.pan, None);
        assert_eq!(initial.effects.omitted, vec![PostSynthesisDimension::Pan]);

        let RuntimeReroute::Retry(retry) = routes.reroute_after_failure(
            &initial,
            &TtsError::VoiceNotFound("primary disappeared".to_owned()),
            &engines,
        ) else {
            panic!("voice failure did not select the alternate engine");
        };
        assert_eq!(retry.realized.engine_id, "pan-only");
        assert_eq!(retry.effects.style.gain, None);
        assert_eq!(retry.effects.style.pan, Some(0.2));
        assert_eq!(retry.effects.omitted, vec![PostSynthesisDimension::Gain]);
    }

    #[test]
    fn voice_failure_uses_an_explicit_alternative_on_the_same_engine() {
        let mut engines = EngineRegistry::new();
        register_engine(&mut engines, "dectalk", &["paul", "betty"]);
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul"), exact("dectalk", "betty")]),
            FallbackPolicy::default(),
        );
        let initial = routes.initial_route("source-code", &engines).unwrap();

        let rerouted = routes.reroute_after_failure(
            &initial,
            &TtsError::VoiceNotFound("Paul was removed".to_owned()),
            &engines,
        );

        let RuntimeReroute::Retry(retry) = rerouted else {
            panic!("voice failure did not select the explicit alternative");
        };
        assert_eq!(MAX_RUNTIME_SYNTHESIS_ATTEMPTS, 4);
        assert_eq!(retry.engine.descriptor().id, "dectalk");
        assert_eq!(retry.realized, PhysicalVoiceId::new("dectalk", "betty"));
    }

    #[test]
    fn engine_failure_uses_the_configured_fallback_engine() {
        let mut engines = EngineRegistry::new();
        register_engine(&mut engines, "dectalk", &["paul"]);
        register_engine(&mut engines, "espeak", &["en-us"]);
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let initial = routes.initial_route("source-code", &engines).unwrap();

        let rerouted = routes.reroute_after_failure(
            &initial,
            &TtsError::SynthesisFailed("helper exited".to_owned()),
            &engines,
        );

        let RuntimeReroute::Retry(retry) = rerouted else {
            panic!("engine failure did not select the fallback engine");
        };
        assert_eq!(retry.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert_eq!(
            routes
                .initial_route("source-code", &engines)
                .unwrap()
                .realized,
            PhysicalVoiceId::new("espeak", "en-us")
        );
        assert!(matches!(
            engines.descriptor("dectalk").unwrap().health,
            EngineHealth::Healthy
        ));
    }

    #[test]
    fn invalid_parameters_are_not_retried_or_marked_failed() {
        let mut engines = EngineRegistry::new();
        register_engine(&mut engines, "dectalk", &["paul"]);
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy::default(),
        );
        let initial = routes.initial_route("source-code", &engines).unwrap();

        assert!(matches!(
            routes.reroute_after_failure(
                &initial,
                &TtsError::InvalidParameter("rate".to_owned()),
                &engines,
            ),
            RuntimeReroute::NotRetryable
        ));
        assert_eq!(
            routes
                .initial_route("source-code", &engines)
                .unwrap()
                .realized,
            initial.realized
        );
    }

    #[test]
    fn fallback_exhaustion_is_explicit() {
        let mut engines = EngineRegistry::new();
        register_engine(&mut engines, "dectalk", &["paul"]);
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy::default(),
        );
        let initial = routes.initial_route("source-code", &engines).unwrap();

        let RuntimeReroute::Exhausted(error) =
            routes.reroute_after_failure(&initial, &TtsError::NotAvailable, &engines)
        else {
            panic!("missing fallback was not reported as exhausted");
        };
        assert!(error.contains("no usable physical voice"));
    }

    #[test]
    fn repertoire_routing_preserves_unicode_text_and_anchors_on_fallback() {
        let helper =
            synthesis_engine_with_repertoire("eloquence", "v1", TextRepertoire::Windows1252);
        let unicode = synthesis_engine_with_repertoire("espeak", "en-us", TextRepertoire::Unicode);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&helper) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&unicode) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("eloquence", "v1")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(11);
        let text = "Élan 日本 👋 e\u{301}";
        let anchors = vec![
            RequestedAnchor::new("start", 0, omnivox_tts::AnchorAffinity::Before),
            RequestedAnchor::new("cjk", 6, omnivox_tts::AnchorAffinity::Before),
            RequestedAnchor::new("end", text.len() as u32, omnivox_tts::AnchorAffinity::After),
        ];

        let outcome = synthesize_with_runtime_fallback_anchored(
            text,
            &anchors,
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            11,
            &counter,
        );

        let RuntimeSynthesisOutcome::Ready(result) = outcome else {
            panic!("Unicode-capable fallback did not synthesize the chunk");
        };
        assert_eq!(
            result.actual_voice,
            Some(PhysicalVoiceId::new("espeak", "en-us"))
        );
        assert_eq!(route.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert!(helper.calls.lock().unwrap().is_empty());
        assert_eq!(
            *unicode.calls.lock().unwrap(),
            [(text.to_owned(), "en-us".to_owned())]
        );
        assert_eq!(*unicode.anchors.lock().unwrap(), [anchors]);
        assert!(matches!(
            routes
                .inventory
                .iter()
                .find(|engine| engine.id == "eloquence")
                .unwrap()
                .health,
            EngineHealth::Healthy
        ));
    }

    #[test]
    fn repertoire_routing_returns_to_preferred_engine_for_compatible_text() {
        let helper =
            synthesis_engine_with_repertoire("eloquence", "v1", TextRepertoire::Windows1252);
        let unicode = synthesis_engine_with_repertoire("espeak", "en-us", TextRepertoire::Unicode);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&helper) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&unicode) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("eloquence", "v1")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(3);

        let unicode_outcome = synthesize_with_runtime_fallback(
            "日本",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            3,
            &counter,
        );
        assert!(matches!(unicode_outcome, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(route.realized.engine_id, "espeak");

        let compatible_outcome = synthesize_with_runtime_fallback(
            "Élan — €",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            3,
            &counter,
        );
        assert!(matches!(
            compatible_outcome,
            RuntimeSynthesisOutcome::Ready(_)
        ));
        assert_eq!(route.realized.engine_id, "eloquence");
        assert_eq!(helper.calls.lock().unwrap().len(), 1);
        assert_eq!(unicode.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn repertoire_routing_fails_without_calling_an_incapable_engine() {
        let helper = synthesis_engine_with_repertoire("dectalk", "paul", TextRepertoire::Iso8859_1);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&helper) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy::default(),
        );
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(5);

        let outcome = synthesize_with_runtime_fallback(
            "emoji 👋",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            5,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Exhausted));
        assert!(helper.calls.lock().unwrap().is_empty());
        assert!(matches!(routes.inventory[0].health, EngineHealth::Healthy));
    }

    #[test]
    fn routed_failure_retries_the_same_chunk_on_the_fallback_engine() {
        let primary = synthesis_engine("dectalk", "paul", Some(MockFailure::Synthesis));
        let fallback = synthesis_engine("espeak", "en-us", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(7);

        let outcome = synthesize_with_runtime_fallback(
            "same chunk",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            7,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(route.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert_eq!(
            *primary.calls.lock().unwrap(),
            [("same chunk".to_owned(), "paul".to_owned())]
        );
        assert_eq!(
            *fallback.calls.lock().unwrap(),
            [("same chunk".to_owned(), "en-us".to_owned())]
        );

        let runtime_inventory = health.snapshot(engines.generation(), engines.inventory());
        let mut next_routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        next_routes.replace_inventory(runtime_inventory.engines);
        assert_eq!(
            next_routes
                .initial_route("source-code", &engines)
                .unwrap()
                .realized,
            PhysicalVoiceId::new("espeak", "en-us")
        );
    }

    #[test]
    fn temporary_unavailability_falls_back_without_opening_a_circuit() {
        let primary = synthesis_engine(
            "eloquence",
            "reed",
            Some(MockFailure::NotAvailableOnce(AtomicUsize::new(1))),
        );
        let fallback = synthesis_engine("espeak", "en-us", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let policy = FallbackPolicy {
            fallback_engines: vec!["espeak".to_owned()],
            ..FallbackPolicy::default()
        };
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(11);
        let mut first_routes = snapshot(
            &engines,
            definition(vec![exact("eloquence", "reed")]),
            policy.clone(),
        );
        let mut first_route = first_routes
            .initial_route("source-code", &engines)
            .unwrap();

        let first = synthesize_with_runtime_fallback(
            "temporarily blocked",
            &TtsSettings::default(),
            &mut first_route,
            &mut first_routes,
            &engines,
            &health,
            11,
            &counter,
        );

        assert!(matches!(first, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(first_route.realized.engine_id, "espeak");
        let runtime_inventory = health.snapshot(engines.generation(), engines.inventory());
        assert!(matches!(
            runtime_inventory
                .engines
                .iter()
                .find(|engine| engine.id == "eloquence")
                .unwrap()
                .health,
            EngineHealth::Healthy
        ));

        let mut next_routes = snapshot(
            &engines,
            definition(vec![exact("eloquence", "reed")]),
            policy,
        );
        next_routes.replace_inventory(runtime_inventory.engines);
        let mut next_route = next_routes
            .initial_route("source-code", &engines)
            .unwrap();
        let next = synthesize_with_runtime_fallback(
            "retry immediately",
            &TtsSettings::default(),
            &mut next_route,
            &mut next_routes,
            &engines,
            &health,
            11,
            &counter,
        );

        assert!(matches!(next, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(next_route.realized.engine_id, "eloquence");
        assert_eq!(primary.calls.lock().unwrap().len(), 2);
        assert_eq!(fallback.calls.lock().unwrap().len(), 1);
        assert_eq!(primary.recovery_preparations.load(Ordering::Acquire), 0);
    }

    #[test]
    fn implicit_legacy_route_falls_back_without_reporting_a_logical_voice() {
        let primary = synthesis_engine("eloquence", "reed", Some(MockFailure::Synthesis));
        let fallback = synthesis_engine("espeak", "en-us", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(Vec::new()),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let mut route = routes
            .initial_legacy_route(PhysicalVoiceId::new("eloquence", "reed"), &engines)
            .unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(3);

        let outcome = synthesize_with_runtime_fallback(
            "plain speech",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            3,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(route.realized, PhysicalVoiceId::new("espeak", "en-us"));
        assert_eq!(*primary.logical_voice_ids.lock().unwrap(), [None]);
        assert_eq!(*fallback.logical_voice_ids.lock().unwrap(), [None]);
    }

    #[test]
    fn successful_recovery_probe_prepares_engine_and_restores_primary_route() {
        let primary = synthesis_engine(
            "dectalk",
            "paul",
            Some(MockFailure::SynthesisOnce(AtomicUsize::new(1))),
        );
        let fallback = synthesis_engine("espeak", "en-us", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let policy = FallbackPolicy {
            fallback_engines: vec!["espeak".to_owned()],
            ..FallbackPolicy::default()
        };
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(9);
        let mut failed_routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            policy.clone(),
        );
        let mut failed_route = failed_routes
            .initial_route("source-code", &engines)
            .unwrap();

        let first = synthesize_with_runtime_fallback(
            "first chunk",
            &TtsSettings::default(),
            &mut failed_route,
            &mut failed_routes,
            &engines,
            &health,
            9,
            &counter,
        );
        assert!(matches!(first, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(failed_route.realized.engine_id, "espeak");

        health.force_probe_ready("dectalk");
        let runtime_inventory = health.snapshot(engines.generation(), engines.inventory());
        let mut probe_routes =
            snapshot(&engines, definition(vec![exact("dectalk", "paul")]), policy);
        probe_routes.replace_inventory(runtime_inventory.engines);
        let mut probe_route = probe_routes.initial_route("source-code", &engines).unwrap();
        assert_eq!(probe_route.realized.engine_id, "dectalk");

        let probe = synthesize_with_runtime_fallback(
            "probe chunk",
            &TtsSettings::default(),
            &mut probe_route,
            &mut probe_routes,
            &engines,
            &health,
            9,
            &counter,
        );

        assert!(matches!(probe, RuntimeSynthesisOutcome::Ready(_)));
        assert_eq!(probe_route.realized.engine_id, "dectalk");
        assert_eq!(primary.recovery_preparations.load(Ordering::Acquire), 1);
        let restored = health.snapshot(engines.generation(), engines.inventory());
        assert!(matches!(restored.engines[0].health, EngineHealth::Healthy));
    }

    #[test]
    fn routed_synthesis_never_exceeds_the_attempt_limit() {
        let mut engines = EngineRegistry::new();
        let mut mocks = Vec::new();
        let mut preferences = Vec::new();
        for index in 0..=MAX_RUNTIME_SYNTHESIS_ATTEMPTS {
            let engine_id = format!("engine-{index}");
            let voice_id = format!("voice-{index}");
            let engine = synthesis_engine(&engine_id, &voice_id, Some(MockFailure::Synthesis));
            engines
                .register(Arc::clone(&engine) as Arc<dyn TtsEngine>)
                .unwrap();
            preferences.push(exact(&engine_id, &voice_id));
            mocks.push(engine);
        }
        let mut routes = snapshot(&engines, definition(preferences), FallbackPolicy::default());
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();
        let counter = AtomicU64::new(3);

        let outcome = synthesize_with_runtime_fallback(
            "bounded chunk",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            3,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Exhausted));
        assert_eq!(
            mocks
                .iter()
                .map(|engine| engine.calls.lock().unwrap().len())
                .sum::<usize>(),
            MAX_RUNTIME_SYNTHESIS_ATTEMPTS
        );
        assert!(mocks[MAX_RUNTIME_SYNTHESIS_ATTEMPTS]
            .calls
            .lock()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cancellation_after_a_failed_attempt_prevents_the_retry() {
        let counter = Arc::new(AtomicU64::new(11));
        let primary = synthesis_engine(
            "dectalk",
            "paul",
            Some(MockFailure::SynthesisAndCancel(Arc::clone(&counter))),
        );
        let fallback = synthesis_engine("espeak", "en-us", None);
        let mut engines = EngineRegistry::new();
        engines
            .register(Arc::clone(&primary) as Arc<dyn TtsEngine>)
            .unwrap();
        engines
            .register(Arc::clone(&fallback) as Arc<dyn TtsEngine>)
            .unwrap();
        let mut routes = snapshot(
            &engines,
            definition(vec![exact("dectalk", "paul")]),
            FallbackPolicy {
                fallback_engines: vec!["espeak".to_owned()],
                ..FallbackPolicy::default()
            },
        );
        let mut route = routes.initial_route("source-code", &engines).unwrap();
        let health = RuntimeEngineHealth::new();

        let outcome = synthesize_with_runtime_fallback(
            "cancelled chunk",
            &TtsSettings::default(),
            &mut route,
            &mut routes,
            &engines,
            &health,
            11,
            &counter,
        );

        assert!(matches!(outcome, RuntimeSynthesisOutcome::Cancelled));
        assert_eq!(primary.calls.lock().unwrap().len(), 1);
        assert!(fallback.calls.lock().unwrap().is_empty());
    }
}
