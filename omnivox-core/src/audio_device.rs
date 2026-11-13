//! Host-scoped output selection, separate from channel routing and engine startup.

use serde::{Deserialize, Serialize};

pub const FEATURE: &str = "audio_device_selection_v1";
pub const MAX_DEVICES: usize = 64;
pub const MAX_DEVICE_ID_BYTES: usize = 1024;
pub const MAX_DEVICE_NAME_BYTES: usize = 512;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceSelection {
    #[default]
    SystemDefault,
    Device {
        id: String,
    },
}

impl DeviceSelection {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let selection = if value == "default" {
            Self::SystemDefault
        } else {
            Self::Device { id: value.into() }
        };
        selection.validate()?;
        Ok(selection)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::Device { id } = self {
            if id.len() > MAX_DEVICE_ID_BYTES
                || id.chars().any(char::is_control)
                || !id
                    .split_once(':')
                    .is_some_and(|(host, device)| !host.is_empty() && !device.is_empty())
            {
                return Err("expected default or a bounded backend-qualified device ID");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceCatalogue {
    pub host_scope: String,
    pub backend: String,
    pub supported: bool,
    pub default_device: Option<String>,
    pub devices: Vec<OutputDevice>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceChange {
    pub sequence: u64,
    pub expected_revision: u64,
    pub selection: DeviceSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceOutcome {
    Preparing,
    Applied,
    Unchanged,
    Rejected,
    Cancelled,
    Busy,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceOperation {
    pub change: DeviceChange,
    pub outcome: DeviceOutcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceState {
    pub revision: u64,
    pub selection: DeviceSelection,
    pub connected: Option<String>,
    pub last_operation: Option<DeviceOperation>,
}

impl DeviceState {
    pub fn new(selection: DeviceSelection) -> Self {
        Self {
            revision: 0,
            selection,
            connected: None,
            last_operation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_default_and_opaque_ids_remain_distinct() {
        assert_eq!(
            DeviceSelection::parse("default"),
            Ok(DeviceSelection::SystemDefault)
        );
        let id = "wasapi:{0.0.0.00000000}.{123}";
        assert_eq!(
            DeviceSelection::parse(id),
            Ok(DeviceSelection::Device { id: id.into() })
        );
        assert!(DeviceSelection::parse("headphones").is_err());
        for id in [
            "",
            ":endpoint",
            "wasapi:",
            "wasapi:bad\nname",
            "wasapi:bad\0name",
        ] {
            assert!(DeviceSelection::parse(id).is_err());
        }
        assert!(
            DeviceSelection::parse(&format!("wasapi:{}", "x".repeat(MAX_DEVICE_ID_BYTES))).is_err()
        );
    }
}
