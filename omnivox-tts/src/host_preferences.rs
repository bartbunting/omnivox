//! Public metadata for local per-worker host preferences; no caller-selected paths.
use serde::{Deserialize, Serialize};

pub const FEATURE: &str = "host_preferences_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReloadPunctuation {
    pub incarnation: String,
    pub sequence: u64,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Preparing,
    Pending,
    Applied,
    Unchanged,
    Rejected,
    Busy,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub sequence: u64,
    pub expected_revision: u64,
    pub outcome: Outcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub incarnation: String,
    pub configuration_file: String,
    pub startup_activation_id: String,
    pub revision: u64,
    pub punctuation_sha256: String,
    pub startup_punctuation_sha256: String,
    pub last_operation: Option<Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReloadResult {
    pub sequence: u64,
    pub outcome: Outcome,
    pub state: Status,
}

#[cfg(test)]
mod tests {
    use crate::control::decode_request;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn local_reload_envelopes_reject_paths_extensions_and_duplicate_keys() {
        let valid = r#"{"protocol_version":1,"request_id":4,"type":"reload_punctuation_v1","incarnation":"worker","sequence":1,"expected_revision":0}"#;
        assert!(decode_request(&STANDARD.encode(valid)).is_ok());
        for extra in [
            r#", "path":"/elsewhere""#,
            r#", "sequence":2"#,
            r#", "expected_revision":null"#,
        ] {
            let modified = format!("{}{extra}}}", valid.strip_suffix('}').unwrap());
            assert!(decode_request(&STANDARD.encode(modified)).is_err());
        }
        assert!(decode_request(&STANDARD.encode(r#"{"protocol_version":1,"request_id":5,"type":"host_preferences_status_v1","path":"/elsewhere"}"#)).is_err());
    }
}
