//! Negotiated local output selection; contains no native handles or file paths.

use omnivox_core::audio_device::{DeviceChange, DeviceOutcome, DeviceSelection, DeviceState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyDevice {
    pub incarnation: String,
    pub change: DeviceChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelDevice {
    pub incarnation: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceStatus {
    pub incarnation: String,
    pub host_scope: String,
    pub state: DeviceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceResult {
    pub sequence: u64,
    pub outcome: DeviceOutcome,
    pub status: DeviceStatus,
}

/// A client's saved choices for one lane. Resolve on the speech host so WSL
/// and native clients need no extra discovery process before ordinary startup.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedDevice {
    host_scope: String,
    selection: DeviceSelection,
}

pub fn saved_selection(json: &str, host_scope: &str) -> Result<DeviceSelection, &'static str> {
    if json.len() > 16_384 {
        return Err("saved audio-device choices exceed 16 KiB");
    }
    serde_json::from_str::<crate::control::DuplicateFreeJson>(json)
        .map_err(|_| "invalid saved audio-device JSON")?;
    let choices: Vec<SavedDevice> =
        serde_json::from_str(json).map_err(|_| "invalid saved audio-device choices")?;
    if choices.len() > 8 {
        return Err("at most eight speech hosts may have saved device choices");
    }
    let mut hosts = std::collections::HashSet::new();
    let mut selection = DeviceSelection::SystemDefault;
    for choice in choices {
        if choice.host_scope.is_empty()
            || choice.host_scope.len() > 512
            || choice.host_scope.chars().any(char::is_control)
            || !hosts.insert(choice.host_scope.clone())
        {
            return Err("invalid or repeated saved audio-device host");
        }
        choice.selection.validate()?;
        if choice.host_scope == host_scope {
            selection = choice.selection;
        }
    }
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use crate::control::decode_request;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn saved_choices_match_only_the_speech_host_and_reject_ambiguous_input() {
        use super::*;
        let json = r#"[{"host_scope":"windows:wasapi:desk","selection":{"mode":"device","id":"wasapi:headphones"}}]"#;
        assert_eq!(
            saved_selection(json, "linux").unwrap(),
            DeviceSelection::SystemDefault
        );
        assert_eq!(
            saved_selection(json, "windows:wasapi:desk").unwrap(),
            DeviceSelection::parse("wasapi:headphones").unwrap()
        );
        for invalid in [
            json.replace(
                "\"mode\":\"device\"",
                "\"mode\":\"device\",\"mode\":\"system_default\"",
            ),
            json.replace("wasapi:headphones", "a friendly name"),
            format!(
                "[{},{}]",
                &json[1..json.len() - 1],
                &json[1..json.len() - 1]
            ),
            format!("[{}]", " ".repeat(16_384)),
        ] {
            assert!(saved_selection(&invalid, "linux").is_err());
        }
    }

    #[test]
    fn selection_envelopes_reject_duplicate_unknown_and_unbounded_fields() {
        let valid = r#"{"protocol_version":1,"request_id":4,"type":"apply_audio_device_v1","incarnation":"worker","change":{"sequence":1,"expected_revision":0,"selection":{"mode":"device","id":"wasapi:headphones"}}}"#;
        assert!(decode_request(&STANDARD.encode(valid)).is_ok());
        for invalid in [
            valid.replace("\"sequence\":1", "\"sequence\":1,\"sequence\":2"),
            valid.replace("\"wasapi:headphones\"", "\"a friendly name\""),
            valid.replace(
                "\"mode\":\"device\"",
                "\"mode\":\"device\",\"fallback\":true",
            ),
            valid.replace("\"worker\"", "\"worker\",\"path\":\"/elsewhere\""),
            valid.replace("wasapi:headphones", &format!("wasapi:{}", "a".repeat(1024))),
        ] {
            assert!(decode_request(&STANDARD.encode(invalid)).is_err());
        }
        for kind in ["audio_devices_v1", "audio_device_status_v1"] {
            let valid = format!(r#"{{"protocol_version":1,"request_id":1,"type":"{kind}"}}"#);
            assert!(decode_request(&STANDARD.encode(&valid)).is_ok());
            assert!(decode_request(&STANDARD.encode(valid.replace(
                "\"request_id\":1",
                "\"request_id\":1,\"path\":\"/elsewhere\""
            )))
            .is_err());
        }
    }
}
