//! One verified voice-library configuration pinned for a server startup.
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use omnivox_tts::contracts::EngineDescriptor;
use omnivox_tts::engine_registry::EngineRegistry;
use omnivox_tts::helper_engine::HelperEngineConfig;
use omnivox_tts::voice_library::{
    HostPlatform, ProviderOverrides, RuntimeLibrary, VoiceEligibility,
};

#[derive(Clone)]
pub(crate) struct StartupLibrary {
    pub path: PathBuf,
    pub library: RuntimeLibrary,
    pub eligibility: Arc<VoiceEligibility>,
    overrides: ProviderOverrides,
}

impl StartupLibrary {
    pub fn from_environment(path: Option<&str>, model: Option<&str>) -> Result<Option<Self>> {
        let path = path
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("OMNIVOX_VOICE_LIBRARY").map(PathBuf::from));
        if path.is_some() {
            anyhow::ensure!(
                model.is_none_or(|value| !value.is_empty()),
                "--piper-model must not be empty when selecting a voice library"
            );
            if model.is_none() {
                anyhow::ensure!(
                    std::env::var_os("OMNIVOX_PIPER_MODEL")
                        .is_none_or(|value| value.to_str().is_some()),
                    "OMNIVOX_PIPER_MODEL is not valid Unicode"
                );
            }
        }
        let overrides = ProviderOverrides {
            piper: model.is_some_and(|s| !s.is_empty())
                || std::env::var_os("OMNIVOX_PIPER_MODEL").is_some_and(|s| !s.is_empty()),
            flite: std::env::var_os("OMNIVOX_FLITE_VOICES").is_some_and(|s| !s.is_empty()),
        };
        path.map(|path| Self::read(path, overrides)).transpose()
    }

    fn read(path: PathBuf, overrides: ProviderOverrides) -> Result<Self> {
        anyhow::ensure!(
            !path.as_os_str().is_empty(),
            "voice-library path must not be empty"
        );
        let path = path
            .canonicalize()
            .context("could not locate voice-library generation")?;
        anyhow::ensure!(
            path.is_file(),
            "voice-library generation must be a regular file"
        );
        let host = if cfg!(windows) {
            HostPlatform::Windows
        } else {
            HostPlatform::Posix
        };
        let library = RuntimeLibrary::read(std::fs::File::open(&path)?, host)?;
        let eligibility = Arc::new(VoiceEligibility::from_library(&library, overrides));
        Ok(Self {
            path,
            library,
            eligibility,
            overrides,
        })
    }

    pub fn manages(&self, engine: &str) -> bool {
        match engine {
            "piper" => self.library.document().piper.is_some() && !self.overrides.piper,
            "flite" => self.library.document().flite.is_some() && !self.overrides.flite,
            "mbrola" => self.library.document().mbrola.is_some(),
            "rhvoice" => self.library.document().rhvoice.is_some(),
            _ => false,
        }
    }

    pub fn requires(&self, engine: &str) -> bool {
        self.manages(engine)
            && !self.eligibility.excludes_provider(engine)
            && (engine != "rhvoice"
                || self
                    .library
                    .document()
                    .rhvoice
                    .as_ref()
                    .is_some_and(|r| !r.voices.is_empty()))
    }

    pub fn configure(&self, config: &mut HelperEngineConfig) {
        if self.manages(&config.engine_id) {
            config.arguments = vec![
                "--voice-library".into(),
                self.path.clone().into_os_string(),
                "--voice-library-sha256".into(),
                self.library.sha256().into(),
            ];
        }
    }

    /// Verify only the provider about to load. A broken optional provider must
    /// not prevent another engine from supplying ordinary fallback speech.
    pub fn verify_assets(&self, engine: &str) -> Result<()> {
        if !self.manages(engine) {
            return Ok(());
        }
        // Reuse the shared verifier, including RHVoice's resource-tree checks.
        // This private projection never replaces the pinned generation passed
        // to helpers or the configuration acknowledged to the client.
        let mut document = self.library.document().clone();
        if engine != "piper" {
            document.piper = None;
        }
        if engine != "flite" {
            document.flite = None;
        }
        if engine != "mbrola" {
            document.mbrola = None;
        }
        if engine != "rhvoice" {
            document.rhvoice = None;
        }
        let host = if cfg!(windows) {
            HostPlatform::Windows
        } else {
            HostPlatform::Posix
        };
        RuntimeLibrary::parse(&serde_json::to_vec(&document)?, host)?
            .verify_assets(ProviderOverrides::default())?;
        Ok(())
    }

    pub fn validate_descriptor(&self, descriptor: &EngineDescriptor) -> Result<()> {
        if !self.manages(&descriptor.id) {
            return Ok(());
        }
        let expected: std::collections::BTreeSet<&str> = match descriptor.id.as_str() {
            "piper" => self
                .library
                .document()
                .piper
                .as_ref()
                .unwrap()
                .models
                .iter()
                .flat_map(|model| model.voices.iter().map(|voice| voice.physical_id.as_str()))
                .collect(),
            "flite" => {
                let flite = self.library.document().flite.as_ref().unwrap();
                flite
                    .builtin_slt
                    .then_some("cmu_us_slt")
                    .into_iter()
                    .chain(flite.files.iter().map(|voice| voice.physical_id.as_str()))
                    .collect()
            }
            "mbrola" => self
                .library
                .document()
                .mbrola
                .as_ref()
                .unwrap()
                .voice_ids()
                .collect(),
            "rhvoice" => self
                .library
                .document()
                .rhvoice
                .as_ref()
                .unwrap()
                .voices
                .iter()
                .map(|v| v.physical_id.as_str())
                .collect(),
            _ => unreachable!(),
        };
        let actual: std::collections::BTreeSet<&str> = descriptor
            .voices
            .iter()
            .map(|voice| voice.id.voice_id.as_str())
            .collect();
        anyhow::ensure!(
            (if descriptor.id == "rhvoice"
                && self
                    .library
                    .document()
                    .rhvoice
                    .as_ref()
                    .unwrap()
                    .inherit_external
            {
                expected.is_subset(&actual)
            } else {
                actual == expected && descriptor.voices.len() == expected.len()
            }) && descriptor.can_synthesize()
                && descriptor
                    .voices
                    .iter()
                    .all(|voice| voice.id.engine_id == descriptor.id
                        && voice.availability.is_available()),
            "{} helper did not provide the complete configured voice set",
            descriptor.id
        );
        Ok(())
    }

    pub fn registry(&self) -> Result<EngineRegistry> {
        let mut registry = EngineRegistry::with_voice_library(&self.library, self.overrides);
        for engine in ["piper", "flite", "mbrola", "rhvoice"] {
            if self.eligibility.excludes_provider(engine) {
                registry.register_unavailable(
                    EngineDescriptor::unavailable(
                        engine,
                        "excluded by voice-library configuration",
                    ),
                    || Err("engine is excluded by configuration".to_owned()),
                )?;
            }
        }
        Ok(registry)
    }
}

/// Check complete initial replies, including non-library engines and Base64.
/// The control codec's encoded bound also fits the remote transport's line bound.
pub(crate) fn preflight_responses(registry: &EngineRegistry, preferred: &str) -> Result<()> {
    use omnivox_tts::control::{
        format_control_event, ControlResponse, ControlResponseEnvelope, CONTROL_PROTOCOL_VERSION,
    };
    use omnivox_tts::routing_policy::RoutingPolicyRegistry;
    let (_, engines) = registry.snapshot();
    let policy = RoutingPolicyRegistry::new(preferred);
    let statuses = crate::health::RuntimeEngineHealth::new().statuses(&engines, &[]);
    let library = registry.voice_library_status(u64::MAX, &engines, &[]);
    for response in [
        ControlResponse::VoiceLibraryStatusV1(library),
        ControlResponse::Inventory {
            inventory_generation: u64::MAX,
            preferred_engine_id: preferred.to_owned(),
            routing_policy: policy.registration(),
            engine_runtime: statuses,
            engines,
        },
    ] {
        let encoded = format_control_event(&ControlResponseEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: Some(u64::MAX),
            response,
        })
        .context("voice-library inventory/status exceeds control transport limits")?;
        anyhow::ensure!(
            encoded.len() < crate::remote::MAX_LINE,
            "voice-library response exceeds remote transport line limit"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
