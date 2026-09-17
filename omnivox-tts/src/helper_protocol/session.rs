//! Parent-side dispatch for negotiated helper 6 and unchanged legacy messages.
//!
//! Shared v6 frames have v5 semantics. Validate those semantics through a private
//! projection; never teach the old codecs to discard new synthesis members.
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{parameters as p, *};

pub(crate) const VERSIONS: &[u16] = &[6, 5, 4, 3, 2, 1];

pub(crate) fn validate_common_request(request: &HelperRequest) -> Result<(), HelperProtocolError> {
    if request.protocol_version != p::PROTOCOL_VERSION {
        return request.validate();
    }
    if matches!(request.body, HelperRequestBody::Synthesize { .. }) {
        return Err(HelperProtocolError::InvalidField("voice_parameters"));
    }
    if let HelperRequestBody::Hello {
        supported_protocol_versions: versions,
    } = &request.body
    {
        validate_request_id(request.request_id)?;
        if versions.is_empty()
            || versions.len() > MAX_SUPPORTED_VERSIONS
            || !versions.contains(&p::PROTOCOL_VERSION)
            || versions
                .iter()
                .enumerate()
                .any(|(i, v)| *v == 0 || versions[..i].contains(v))
        {
            return Err(HelperProtocolError::InvalidField(
                "supported_protocol_versions",
            ));
        }
        return Ok(());
    }
    let mut projected = request.clone();
    projected.protocol_version = HELPER_PROTOCOL_V5;
    projected.validate()
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Response {
    Common(HelperResponse),
    Parameters(p::Response),
}

impl Response {
    pub(crate) fn version(&self) -> u16 {
        match self {
            Self::Common(r) => r.protocol_version,
            Self::Parameters(r) => r.protocol_version,
        }
    }
    pub(crate) fn request_id(&self) -> Option<u64> {
        match self {
            Self::Common(r) => r.request_id,
            Self::Parameters(r) => Some(r.request_id),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), HelperProtocolError> {
        match self {
            Self::Parameters(r) => r.validate(),
            Self::Common(r) if r.protocol_version == p::PROTOCOL_VERSION => {
                if matches!(r.body, HelperResponseBody::SynthesisStarted { .. }) {
                    return Err(HelperProtocolError::InvalidField("native_application"));
                }
                let mut projected = r.clone();
                projected.protocol_version = HELPER_PROTOCOL_V5;
                if let HelperResponseBody::Hello {
                    selected_protocol_version,
                    ..
                } = &mut projected.body
                {
                    if *selected_protocol_version != p::PROTOCOL_VERSION {
                        return Err(HelperProtocolError::InvalidField(
                            "selected_protocol_version",
                        ));
                    }
                    *selected_protocol_version = HELPER_PROTOCOL_V5;
                }
                projected.validate()
            }
            Self::Common(r) => r.validate(),
        }
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (version, id, mut fields): (u16, Option<u64>, _) = super::wire::envelope(d, true)?;
        fields.insert("protocol_version".into(), version.into());
        fields.insert("request_id".into(), id.into());
        let native = version == p::PROTOCOL_VERSION
            && matches!(
                fields.get("type").and_then(Value::as_str),
                Some(
                    "synthesis_started" | "engine_parameters_v1" | "voice_parameters_explained_v1"
                )
            );
        let result = if native {
            Self::Parameters(serde_json::from_value(fields.into()).map_err(D::Error::custom)?)
        } else {
            Self::Common(serde_json::from_value(fields.into()).map_err(D::Error::custom)?)
        };
        result.validate().map_err(D::Error::custom)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v6_shared_frames_retain_strict_envelopes_and_required_native_start() {
        for value in [
            json!({"protocol_version":6,"request_id":1,"type":"hello","selected_protocol_version":6,"helper_name":"test","helper_version":"1"}),
            json!({"protocol_version":6,"request_id":1,"type":"audio_chunk","chunk":{"sequence":0,"data_base64":"AAA="}}),
            json!({"protocol_version":6,"request_id":1,"type":"synthesis_completed","frame_count":1}),
            json!({"protocol_version":6,"request_id":1,"type":"cancel_accepted","target_request_id":2}),
            json!({"protocol_version":6,"request_id":1,"type":"pong"}),
        ] {
            let text = serde_json::to_string(&value).unwrap();
            let response: Response = serde_json::from_str(&text).unwrap();
            assert_eq!(response.version(), 6);
            assert!(serde_json::from_str::<HelperResponse>(&text)
                .unwrap()
                .validate()
                .is_err());
            let mut extra = value;
            extra["native_application"] = Value::Null;
            assert!(serde_json::from_value::<Response>(extra).is_err());
        }
        let start = json!({"protocol_version":6,"request_id":1,"type":"synthesis_started",
            "format":{"sample_rate":11025,"channels":1,"sample_format":"pcm_s16_le"},"actual_voice_id":"reed","native_application":null});
        assert!(serde_json::from_value::<Response>(start.clone()).is_ok());
        let mut absent = start.clone();
        absent.as_object_mut().unwrap().remove("native_application");
        assert!(serde_json::from_value::<Response>(absent).is_err());
        let mut legacy = start;
        legacy["protocol_version"] = 5.into();
        assert!(serde_json::from_value::<Response>(legacy).is_err());
        for raw in [
            r#"{"protocol_version":6,"request_id":1,"type":"pong","type":"pong"}"#,
            r#"{"protocol_version":6,"request_id":1,"type":"synthesis_started","format":{"sample_rate":11025,"sample_rate":22050,"channels":1,"sample_format":"pcm_s16_le"},"actual_voice_id":"reed","native_application":null}"#,
            r#"{"protocol_version":6,"request_id":1,"type":"hello","selected_protocol_version":5,"helper_name":"test","helper_version":"1"}"#,
            r#"{"protocol_version":7,"request_id":1,"type":"pong"}"#,
        ] {
            assert!(serde_json::from_str::<Response>(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn v6_control_requests_cannot_accidentally_send_legacy_synthesis() {
        let hello = |versions| {
            HelperRequest::with_version(
                6,
                1,
                HelperRequestBody::Hello {
                    supported_protocol_versions: versions,
                },
            )
        };
        assert!(validate_common_request(&hello(vec![6, 5, 4, 3, 2, 1])).is_ok());
        assert!(validate_common_request(&hello(vec![6])).is_ok());
        for versions in [vec![5], vec![6, 6], vec![6, 0], vec![6; 17]] {
            assert!(validate_common_request(&hello(versions)).is_err());
        }
        let synth = HelperRequest::with_version(
            6,
            2,
            HelperRequestBody::Synthesize {
                text: "hello".into(),
                settings: HelperSynthesisSettings {
                    voice_id: None,
                    rate: 0.5,
                    pitch: 1.0,
                    volume: 1.0,
                    pitch_range: None,
                    stress: None,
                    richness: None,
                },
                anchors: Some(vec![]),
            },
        );
        assert!(validate_common_request(&synth).is_err());
    }
}
