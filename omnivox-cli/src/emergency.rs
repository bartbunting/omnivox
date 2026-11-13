//! Explicit minimal startup, independent of preferences and management storage.
use anyhow::Result;
use omnivox_tts::engine_configuration::LaunchEnvironment;
use omnivox_tts::voice_library::local::Startup;
use std::ffi::OsStr;

pub fn worker() -> bool {
    std::env::var_os("OMNIVOX_EMERGENCY_WORKER").is_some()
}

/// Opt-in startup spans; never instruments ordinary speech admission.
pub fn timed<T>(stage: &str, work: impl FnOnce() -> T) -> T {
    if std::env::var_os("OMNIVOX_STARTUP_TIMING").as_deref() != Some(OsStr::new("1")) {
        return work();
    }
    let start = std::time::Instant::now();
    let result = work();
    eprintln!(
        "OMNIVOX-STARTUP {}",
        serde_json::json!({"pid": std::process::id(), "stage": stage, "elapsed_us": start.elapsed().as_micros()})
    );
    result
}

fn retain_environment(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    !name.starts_with('=')
        && !name.starts_with("OMNIVOX_")
        && !name.starts_with("SWIFTMAC_")
        && !name.starts_with("SHARPWIN_")
        && !matches!(
            name.as_str(),
            "ESPEAK_NG_DATA" | "ALSA_DEFAULT" | "PULSE_SINK"
        )
}

pub fn startup(output: &str, data: Option<&std::path::Path>) -> Result<Startup> {
    anyhow::ensure!(
        data.is_none_or(std::path::Path::is_absolute),
        "emergency data needs an absolute native path"
    );
    let environment = LaunchEnvironment::from_variables(
        std::env::vars_os().filter(|(name, _)| retain_environment(name)),
    )
    .with_variable("OMNIVOX_EMERGENCY_WORKER", Some("1".into()))
    .with_variable(
        "OMNIVOX_EMERGENCY_DATA",
        data.map(|path| path.as_os_str().to_owned()),
    );
    Ok(Startup::emergency(environment, output)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::engine_configuration::LaunchSnapshot;

    #[test]
    fn emergency_removes_preference_and_engine_overrides_but_keeps_host_io() {
        for key in [
            "OMNIVOX_CONFIG_DIR",
            "omnivox_piper_helper",
            "OMNIVOX_VOICE_ROOT",
            "ESPEAK_NG_DATA",
            "SWIFTMAC_AUDIO_TARGET",
            "SHARPWIN_AUDIO_TARGET",
            "ALSA_DEFAULT",
            "PULSE_SINK",
            "=C:",
        ] {
            assert!(!retain_environment(OsStr::new(key)), "{key}");
        }
        for key in [
            "SystemRoot",
            "PATH",
            "HOME",
            "PULSE_SERVER",
            "XDG_RUNTIME_DIR",
        ] {
            assert!(retain_environment(OsStr::new(key)), "{key}");
        }
        assert!(startup("unsupported", None).is_err());
        assert!(startup("null", Some(std::path::Path::new("relative"))).is_err());
    }

    #[test]
    fn emergency_snapshot_cannot_construct_optional_engines_or_reload_files() {
        let startup = startup("null", None).unwrap();
        assert!(startup.configuration.is_none());
        let snapshot = startup.engines.unwrap();
        assert!(snapshot.managed().is_none());
        assert!(snapshot.resolved().root.is_none());
        let restored = LaunchSnapshot::parse(&snapshot.to_bytes().unwrap()).unwrap();
        let available: Vec<_> = restored
            .resolved()
            .registrations()
            .filter(|registration| registration.unavailable.is_none())
            .map(|registration| registration.engine_id.as_str())
            .collect();
        assert_eq!(available, ["espeak"]);
        assert!(restored
            .resolved()
            .registrations()
            .all(|registration| registration.helper.is_none()));
    }
}
