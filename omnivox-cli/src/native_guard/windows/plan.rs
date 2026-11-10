//! Private launch authority captured before the owner can retire old speech.
//! The identity covers the resolved invocation, not executable bytes. Existing
//! payload verification, worker managed-asset checks and helper handshake remain
//! in place. Plan preparation does no filesystem I/O on the guard control thread.
use super::helper_wire;
use anyhow::{Context, Result};
use omnivox_tts::engine_configuration::EngineOrigin;
use omnivox_tts::helper_engine::HelperEngineConfig;
use omnivox_tts::voice_library::local::Startup;

#[derive(Clone)]
pub(super) struct Helper {
    pub config: HelperEngineConfig,
    pub identity: String,
}
#[derive(Clone)]
pub(super) struct Plan {
    pub helper: Option<Helper>,
}

impl Plan {
    pub fn validate(startup: &Startup) -> Result<()> {
        let snapshot = startup
            .engines
            .as_ref()
            .context("guard requires an engine snapshot")?;
        for entry in snapshot
            .resolved()
            .registrations()
            .filter(|entry| entry.enabled)
        {
            anyhow::ensure!(
                matches!(entry.engine_id.as_str(), "espeak" | "flite")
                    && entry.origin != EngineOrigin::ExternalHelper,
                "Windows guarded startup currently supports explicit eSpeak/Flite configurations; unsupported engine: {}",
                entry.engine_id
            );
        }
        Ok(())
    }
    pub fn prepare(startup: &Startup) -> Result<Self> {
        Self::validate(startup)?;
        let helper = startup
            .engines
            .as_ref()
            .unwrap()
            .resolved()
            .registration("flite")
            .filter(|entry| entry.enabled && entry.unavailable.is_none())
            .and_then(|entry| entry.helper.clone())
            .map(Helper::capture)
            .transpose()?;
        Ok(Self { helper })
    }
}

impl Helper {
    pub(super) fn capture(config: HelperEngineConfig) -> Result<Self> {
        let identity = helper_wire::identity(
            &config.engine_id,
            &config.program,
            &config.arguments,
            &config.environment,
        )?;
        anyhow::ensure!(
            config.program.is_absolute(),
            "guard helper requires an absolute program"
        );
        Ok(Self { config, identity })
    }
}
