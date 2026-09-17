use super::super::{HelperProtocolError, HelperRequest, HelperRequestBody, HELPER_PROTOCOL_V5};
use super::*;
use crate::native_parameters::{
    CatalogueIdentity, DefaultSource, ValueOrigin, MAX_NATIVE_OPERATIONS, MAX_PARAMETERS,
};
use serde::Serialize;
use std::collections::BTreeSet;

pub(super) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
pub(super) fn text(s: &str, limit: usize) -> bool {
    !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control)
}
fn voice(s: &str) -> bool {
    text(s, crate::logical_voices::MAX_PHYSICAL_VOICE_ID_BYTES)
}
fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic())
}
fn revision(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identity(i: &CatalogueIdentity) -> Result<(), HelperProtocolError> {
    i.validate().map_err(|_| invalid("identity"))
}
fn bounded<T: Serialize>(value: &T) -> Result<(), HelperProtocolError> {
    require(
        serde_json::to_vec(value)?.len() <= crate::control::MAX_CONTROL_PAYLOAD_BYTES,
        "parameter_payload_bytes",
    )
}
fn ids(values: &[String], max: usize) -> bool {
    values.len() <= max
        && values.iter().all(|v| identifier(v))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn header(version: u16, request_id: u64) -> Result<(), HelperProtocolError> {
    if version != PROTOCOL_VERSION {
        return Err(HelperProtocolError::UnsupportedVersion(version));
    }
    super::super::validate_request_id(request_id)
}
impl VoiceParameters {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        self.native
            .validate_shape()
            .map_err(|_| invalid("native"))?;
        identity(&self.expected_identity)?;
        require(
            self.context_dimensions.len() <= 14
                && self
                    .context_dimensions
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.context_dimensions.len(),
            "context_dimensions",
        )?;
        bounded(self)
    }
}
impl CatalogueQuery {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        require(identifier(&self.engine_id), "engine_id")?;
        require(self.voice_id.as_deref().is_none_or(voice), "voice_id")?;
        require(self.cursor.as_deref().is_none_or(token), "cursor")?;
        require(
            self.expected_catalogue_revision
                .as_deref()
                .is_none_or(revision)
                && (self.cursor.is_none() || self.expected_catalogue_revision.is_some()),
            "expected_catalogue_revision",
        )
    }
}
impl Request {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        header(self.protocol_version, self.request_id)?;
        match &self.body {
            RequestBody::Synthesize {
                text,
                settings,
                anchors,
                voice_parameters,
            } => {
                // Reuse established common, text and anchor semantics, without
                // changing any supported legacy request or negotiating version 6.
                HelperRequest::with_version(
                    HELPER_PROTOCOL_V5,
                    self.request_id,
                    HelperRequestBody::Synthesize {
                        text: text.clone(),
                        settings: settings.clone(),
                        anchors: Some(anchors.clone()),
                    },
                )
                .validate()?;
                if let Some(parameters) = voice_parameters {
                    parameters.validate()?;
                }
                require(
                    serde_json::to_vec(self)?.len() <= super::super::MAX_HELPER_FRAME_BYTES,
                    "frame_bytes",
                )
            }
            RequestBody::GetEngineParametersV1(query) => query.validate(),
            RequestBody::ExplainVoiceParametersV1 { source } => {
                match source {
                    ExplanationSource::Draft {
                        settings,
                        voice_parameters,
                    } => {
                        settings.validate(PROTOCOL_VERSION)?;
                        if let Some(parameters) = voice_parameters {
                            parameters.validate()?;
                        }
                    }
                    ExplanationSource::Applied { plan_id } => {
                        require(identifier(plan_id), "plan_id")?
                    }
                }
                bounded(self)
            }
        }
    }
}
impl CatalogueResult {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        match self {
            Self::Ready {
                identity: i,
                voice_id,
                parameters,
                mappings,
                next_cursor,
            } => {
                identity(i)?;
                require(voice_id.as_deref().is_none_or(voice), "voice_id")?;
                require(
                    parameters.len() <= MAX_PAGE_PARAMETERS
                        && (next_cursor.is_none() || !parameters.is_empty()),
                    "parameters",
                )?;
                let mut seen = BTreeSet::new();
                for p in parameters {
                    p.validate().map_err(|_| invalid("parameter_descriptor"))?;
                    require(seen.insert(&p.id), "duplicate_parameter")?;
                    require(
                        voice_id.is_some() || p.default.source != DefaultSource::RuntimeReadback,
                        "voice_default",
                    )?;
                }
                require(mappings.len() <= MAX_PARAMETERS, "mappings")?;
                for m in mappings {
                    require(
                        !m.common_inputs.is_empty()
                            && m.common_inputs.len() <= 14
                            && m.common_inputs.iter().collect::<BTreeSet<_>>().len()
                                == m.common_inputs.len(),
                        "common_inputs",
                    )?;
                    // Mapping outputs/side effects may name another page. Full
                    // catalogue validation belongs to CatalogueAssembly::finish.
                    require(
                        !m.native_outputs.is_empty() && ids(&m.native_outputs, MAX_PARAMETERS),
                        "native_outputs",
                    )?;
                }
                require(next_cursor.as_deref().is_none_or(token), "next_cursor")?;
            }
            Self::Busy { retry_after_ms } => {
                require((1..=5000).contains(retry_after_ms), "retry_after_ms")?
            }
            Self::Unavailable { message, .. } => require(text(message, 1024), "message")?,
        }
        bounded(self)
    }
}
impl NativeApplication {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        require(
            ids(&self.masked_parameters, MAX_NATIVE_OPERATIONS),
            "masked_parameters",
        )?;
        if let Some(i) = &self.identity {
            identity(i)?;
        }
        match self.status {
            ApplicationStatus::Applied => require(
                self.plan_id.as_deref().is_some_and(identifier)
                    && self.identity.is_some()
                    && self.reason.is_none(),
                "native_application",
            )?,
            ApplicationStatus::CommonOnly => require(
                self.plan_id.is_none()
                    && self.masked_parameters.is_empty()
                    && self.reason.as_deref().is_some_and(|s| text(s, 1024)),
                "native_application",
            )?,
        }
        bounded(self)
    }
}
impl ExplanationResult {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        match self {
            Self::Ready {
                evidence,
                plan_id,
                realized,
                identity: i,
                parameters,
            } => {
                identity(i)?;
                require(
                    identifier(&realized.engine_id) && voice(&realized.voice_id),
                    "realized",
                )?;
                require(
                    match evidence {
                        Evidence::Planned => plan_id.is_none(),
                        Evidence::AdapterApplied => plan_id.as_deref().is_some_and(identifier),
                    },
                    "plan_id",
                )?;
                require(parameters.len() <= MAX_NATIVE_OPERATIONS, "parameters")?;
                let mut seen = BTreeSet::new();
                for p in parameters {
                    require(identifier(&p.id) && seen.insert(&p.id), "parameter_id")?;
                    if let Some(v) = &p.value {
                        v.validate().map_err(|_| invalid("value"))?;
                    }
                    require(
                        !p.read_back
                            || (*evidence == Evidence::AdapterApplied && p.value.is_some()),
                        "read_back",
                    )?;
                    require(
                        !p.masked_native || p.origin == ValueOrigin::ContextMapping,
                        "masked_native",
                    )?;
                }
            }
            Self::Busy { retry_after_ms } => {
                require((1..=5000).contains(retry_after_ms), "retry_after_ms")?
            }
            Self::Unavailable { message, .. } => require(text(message, 1024), "message")?,
        }
        bounded(self)
    }
}
impl Response {
    pub fn validate(&self) -> Result<(), HelperProtocolError> {
        header(self.protocol_version, self.request_id)?;
        match &self.body {
            ResponseBody::SynthesisStarted {
                format,
                actual_voice_id,
                native_application,
            } => {
                format.validate()?;
                require(voice(actual_voice_id), "actual_voice_id")?;
                if let Some(application) = native_application {
                    application.validate()?;
                }
            }
            ResponseBody::EngineParametersV1 { engine_id, result } => {
                require(identifier(engine_id), "engine_id")?;
                result.validate()?;
            }
            ResponseBody::VoiceParametersExplainedV1 { result } => result.validate()?,
        }
        bounded(self)
    }

    /// Check correlation and evidence against the exact submitted request and
    /// helper engine. This validates claims, not native execution or PCM ordering.
    pub fn validate_for(
        &self,
        request: &Request,
        engine_id: &str,
    ) -> Result<(), HelperProtocolError> {
        request.validate()?;
        self.validate()?;
        require(identifier(engine_id), "engine_id")?;
        require(self.request_id == request.request_id, "request_id")?;
        match (&request.body, &self.body) {
            (
                RequestBody::GetEngineParametersV1(query),
                ResponseBody::EngineParametersV1 {
                    engine_id: reported,
                    result,
                },
            ) => {
                require(
                    reported == engine_id && query.engine_id == engine_id,
                    "engine_id",
                )?;
                if let CatalogueResult::Ready {
                    identity, voice_id, ..
                } = result
                {
                    require(voice_id == &query.voice_id, "voice_id")?;
                    require(
                        query
                            .expected_catalogue_revision
                            .as_ref()
                            .is_none_or(|r| r == &identity.catalogue_revision),
                        "catalogue_revision",
                    )?;
                }
            }
            (
                RequestBody::Synthesize {
                    settings,
                    voice_parameters,
                    ..
                },
                ResponseBody::SynthesisStarted {
                    actual_voice_id,
                    native_application,
                    ..
                },
            ) => {
                require(
                    settings
                        .voice_id
                        .as_ref()
                        .is_none_or(|v| v == actual_voice_id),
                    "actual_voice_id",
                )?;
                match (voice_parameters, native_application) {
                    (None, None) => {}
                    (Some(p), Some(a)) => {
                        if a.status == ApplicationStatus::Applied {
                            require(
                                p.native.engine_id == engine_id
                                    && p.native.schema_id == p.expected_identity.schema_id
                                    && a.identity.as_ref() == Some(&p.expected_identity),
                                "identity",
                            )?;
                            require(
                                a.masked_parameters
                                    .iter()
                                    .all(|id| p.native.parameters.contains_key(id)),
                                "masked_parameters",
                            )?;
                        } else {
                            require(
                                p.unavailable_policy == UnavailablePolicy::CommonOnly,
                                "unavailable_policy",
                            )?;
                        }
                    }
                    _ => return Err(invalid("native_application")),
                }
            }
            (
                RequestBody::ExplainVoiceParametersV1 { source },
                ResponseBody::VoiceParametersExplainedV1 { result },
            ) => {
                if let ExplanationResult::Ready {
                    evidence,
                    plan_id,
                    realized,
                    identity,
                    ..
                } = result
                {
                    require(realized.engine_id == engine_id, "realized.engine_id")?;
                    match source {
                        ExplanationSource::Applied { plan_id: requested } => require(
                            *evidence == Evidence::AdapterApplied
                                && plan_id.as_ref() == Some(requested),
                            "plan_id",
                        )?,
                        ExplanationSource::Draft {
                            settings,
                            voice_parameters,
                        } => {
                            require(
                                *evidence == Evidence::Planned
                                    && settings
                                        .voice_id
                                        .as_ref()
                                        .is_none_or(|v| v == &realized.voice_id),
                                "planned_voice",
                            )?;
                            if let Some(p) = voice_parameters {
                                require(
                                    p.native.engine_id == engine_id
                                        && p.native.schema_id == identity.schema_id
                                        && p.expected_identity == *identity,
                                    "identity",
                                )?;
                            }
                        }
                    }
                }
            }
            _ => return Err(invalid("response_type")),
        }
        Ok(())
    }
}
