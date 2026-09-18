//! Explicit native synthesis through the shared engine abstraction.
//!
//! Receipts describe tentative adapter application, not PCM commitment or playback.
pub use crate::helper_protocol::parameters::{
    ApplicationStatus, NativeApplication, UnavailablePolicy, VoiceParameters,
};
use crate::TtsError;

pub fn validate_request(parameters: &VoiceParameters) -> Result<(), TtsError> {
    parameters
        .validate()
        .map_err(|e| TtsError::InvalidParameter(e.to_string()))
}

/// Unsupported adapters may use common settings only when explicitly permitted.
/// Validate even degraded requests; malformed native data never becomes speech.
pub fn common_only(parameters: &VoiceParameters) -> Result<NativeApplication, TtsError> {
    validate_request(parameters)?;
    if parameters.unavailable_policy != UnavailablePolicy::CommonOnly {
        return Err(TtsError::InvalidParameter(
            "This engine cannot execute native voice parameters".into(),
        ));
    }
    Ok(NativeApplication {
        status: ApplicationStatus::CommonOnly,
        plan_id: None,
        identity: None,
        masked_parameters: vec![],
        reason: Some("This engine does not support native voice parameters".into()),
    })
}

/// Check an acknowledgement before it crosses an execution or playback boundary.
pub fn validate_application(
    engine_id: &str,
    parameters: &VoiceParameters,
    application: &NativeApplication,
) -> Result<(), TtsError> {
    application
        .validate()
        .map_err(|e| TtsError::SynthesisFailed(e.to_string()))?;
    let valid = match application.status {
        ApplicationStatus::Applied => {
            parameters.native.engine_id == engine_id
                && parameters.native.schema_id == parameters.expected_identity.schema_id
                && application.identity.as_ref() == Some(&parameters.expected_identity)
                && application
                    .masked_parameters
                    .iter()
                    .all(|id| parameters.native.parameters.contains_key(id))
        }
        ApplicationStatus::CommonOnly => {
            parameters.unavailable_policy == UnavailablePolicy::CommonOnly
        }
    };
    if !valid {
        return Err(TtsError::SynthesisFailed(
            "Native application does not match the synthesis request".into(),
        ));
    }
    Ok(())
}
