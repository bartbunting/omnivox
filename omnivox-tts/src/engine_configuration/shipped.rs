//! Canonical identities are reserved even when a platform or feature is absent.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{LaunchEnvironment, Platform};
use crate::helper_engine::HelperEngineConfig;

pub const ALIASES: &[&str] = &["native"];

pub struct ShippedEngine {
    pub id: &'static str,
    pub in_process: bool,
    pub helper_environment: Option<&'static str>,
}

impl ShippedEngine {
    /// Verified installation-relative candidates only. MBROLA remains explicit.
    pub fn helper_candidates(&self, platform: Platform) -> Vec<PathBuf> {
        if self.in_process || self.id == "mbrola" {
            return Vec::new();
        }
        if matches!(self.id, "eloquence" | "dectalk") {
            match platform {
                Platform::MacOs => return Vec::new(),
                Platform::Windows => {
                    return vec![PathBuf::from(if self.id == "eloquence" {
                        "OmnivoxEloquenceHelper32.exe"
                    } else {
                        "OmnivoxDectalkHelper32.exe"
                    })]
                }
                Platform::Unix => (),
            }
        }
        let suffix = if platform == Platform::Windows {
            ".exe"
        } else {
            ""
        };
        let filename = format!("omnivox-{}-helper{suffix}", self.id);
        vec![
            PathBuf::from(self.id).join(&filename),
            PathBuf::from(filename),
        ]
    }

    pub fn synthesis_idle_timeout(&self) -> Duration {
        // Retain the qualified early failure of a wedged legacy ECI call.
        Duration::from_millis(if self.id == "eloquence" { 500 } else { 60_000 })
    }

    pub fn helper_config(
        &self,
        executable: &Path,
        platform: Platform,
        environment: &LaunchEnvironment,
    ) -> Option<HelperEngineConfig> {
        let variable = self.helper_environment?;
        let program = match environment.get(variable).filter(|value| !value.is_empty()) {
            Some(program) => PathBuf::from(program),
            None => resolve_adjacent(executable, &self.helper_candidates(platform))?,
        };
        let mut config =
            HelperEngineConfig::with_environment(self.id, program, environment.clone());
        config.synthesis_idle_timeout = self.synthesis_idle_timeout();
        Some(config)
    }
}

pub fn resolve_adjacent(executable: &Path, candidates: &[PathBuf]) -> Option<PathBuf> {
    let directory = executable.parent()?;
    candidates
        .iter()
        .map(|candidate| directory.join(candidate))
        .find(|candidate| candidate.is_file())
}

pub const ENGINES: &[ShippedEngine] = &[
    ShippedEngine {
        id: "espeak",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "winrt",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "macos",
        in_process: true,
        helper_environment: None,
    },
    ShippedEngine {
        id: "piper",
        in_process: false,
        helper_environment: Some("OMNIVOX_PIPER_HELPER"),
    },
    ShippedEngine {
        id: "rhvoice",
        in_process: false,
        helper_environment: Some("OMNIVOX_RHVOICE_HELPER"),
    },
    ShippedEngine {
        id: "flite",
        in_process: false,
        helper_environment: Some("OMNIVOX_FLITE_HELPER"),
    },
    ShippedEngine {
        id: "rutts",
        in_process: false,
        helper_environment: Some("OMNIVOX_RUTTS_HELPER"),
    },
    ShippedEngine {
        id: "tgspeechbox",
        in_process: false,
        helper_environment: Some("OMNIVOX_TGSPEECHBOX_HELPER"),
    },
    ShippedEngine {
        id: "eloquence",
        in_process: false,
        helper_environment: Some("OMNIVOX_ELOQUENCE_HELPER"),
    },
    ShippedEngine {
        id: "dectalk",
        in_process: false,
        helper_environment: Some("OMNIVOX_DECTALK_HELPER"),
    },
    ShippedEngine {
        id: "mbrola",
        in_process: false,
        helper_environment: Some("OMNIVOX_MBROLA_HELPER"),
    },
];

pub fn definition(id: &str) -> Option<&'static ShippedEngine> {
    ENGINES.iter().find(|engine| engine.id == id)
}

pub fn reserved(id: &str) -> bool {
    definition(id).is_some() || ALIASES.contains(&id)
}
