//! Complete private startup records. Parsing does not consult configuration
//! files, the current environment, adjacent executables or native engines.
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::*;
use crate::helper_engine::HelperEngineConfig;
use crate::voice_library::{HostPlatform, ProviderOverrides, RuntimeLibrary};

/// Includes native-string encoding and the exact managed generation. Individual
/// configuration/manifest limits remain enforced at their original read boundary.
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct ManagedLaunch {
    pub path: PathBuf,
    pub library: RuntimeLibrary,
    pub overrides: ProviderOverrides,
}

/// An owned activation, cloned for each lane and every recovery attempt. Its
/// UUID identifies configuration ownership; it does not attest executable bytes.
/// Intentionally no Debug implementation: paths, argv and environment are private.
#[derive(Clone)]
pub struct LaunchSnapshot {
    schema: u32,
    activation_id: String,
    resolved: ResolvedConfiguration,
    managed: Option<ManagedLaunch>,
    requested: String,
    piper_selected: bool,
}

impl LaunchSnapshot {
    pub fn prepare(
        resolved: ResolvedConfiguration,
        managed: Option<ManagedLaunch>,
        requested: String,
        piper_selected: bool,
    ) -> Result<Self> {
        for registration in resolved.registrations() {
            if let Some(helper) = &registration.helper {
                require(
                    helper.environment == resolved.environment,
                    "helper environment differs from snapshot",
                )?;
                require(
                    [
                        helper.startup_timeout,
                        helper.request_timeout,
                        helper.synthesis_idle_timeout,
                    ]
                    .iter()
                    .all(|timeout| timeout.subsec_nanos() % 1_000_000 == 0),
                    "snapshot timeout is not an integer millisecond",
                )?;
            }
        }
        let snapshot = Self {
            schema: 7,
            activation_id: crate::voice_library::local::new_uuid()
                .map_err(|_| invalid("could not create activation identity"))?,
            resolved,
            managed,
            requested,
            piper_selected,
        };
        // Validate the same complete representation that workers consume.
        Self::parse(&snapshot.to_bytes()?)?;
        Ok(snapshot)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value = json::parse_snapshot(bytes, MAX_SNAPSHOT_BYTES)?;
        serde_json::from_value(value).map_err(|_| invalid("invalid or incomplete launch snapshot"))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes =
            serde_json::to_vec(self).map_err(|_| invalid("could not encode launch snapshot"))?;
        require(
            bytes.len() <= MAX_SNAPSHOT_BYTES,
            "launch snapshot exceeds byte limit",
        )?;
        Ok(bytes)
    }

    pub fn activation_id(&self) -> &str {
        &self.activation_id
    }
    pub fn resolved(&self) -> &ResolvedConfiguration {
        &self.resolved
    }
    pub fn managed(&self) -> Option<&ManagedLaunch> {
        self.managed.as_ref()
    }
    pub fn requested(&self) -> &str {
        &self.requested
    }
    pub fn piper_selected(&self) -> bool {
        self.piper_selected
    }
}

impl Serialize for LaunchSnapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        SnapshotWire::from(self).serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for LaunchSnapshot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        SnapshotWire::deserialize(deserializer)
            .map_err(|_| serde::de::Error::custom("invalid or incomplete launch snapshot"))?
            .restore()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema: u32,
    #[serde(
        default,
        deserialize_with = "present_speech",
        skip_serializing_if = "Option::is_none"
    )]
    speech: Option<ChunkSpeechWire>,
    #[serde(
        default,
        deserialize_with = "present_defaults",
        skip_serializing_if = "Option::is_none"
    )]
    speech_defaults: Option<SpeechDefaults>,
    #[serde(
        default,
        deserialize_with = "present_capital_pitch",
        skip_serializing_if = "Option::is_none"
    )]
    capital_pitch: Option<CapitalPitchSettings>,
    #[serde(
        default,
        deserialize_with = "present_audio",
        skip_serializing_if = "Option::is_none"
    )]
    audio: Option<AudioOutputSettings>,
    #[serde(
        default,
        deserialize_with = "present_punctuation",
        skip_serializing_if = "Option::is_none"
    )]
    punctuation: Option<PunctuationTables>,
    platform: String,
    activation_id: String,
    registrations: Vec<RegistrationWire>,
    routing: RoutingWire,
    // Native strings preserve non-Unicode values. A sequence also permits
    // duplicate detection before collecting into the immutable environment.
    environment: Vec<(OsString, OsString)>,
    #[serde(deserialize_with = "required_nullable")]
    root: Option<OsString>,
    #[serde(deserialize_with = "required_nullable")]
    managed: Option<ManagedWire>,
    requested: String,
    piper_selected: bool,
}

fn present_speech<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ChunkSpeechWire>, D::Error> {
    ChunkSpeechWire::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkSpeechWire {
    max_chunk_words: ChunkWordLimit,
}

fn present_defaults<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<SpeechDefaults>, D::Error> {
    let defaults = SpeechDefaults::deserialize(deserializer)?;
    defaults.validate().map_err(serde::de::Error::custom)?;
    Ok(Some(defaults))
}

fn present_capital_pitch<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<CapitalPitchSettings>, D::Error> {
    let settings = CapitalPitchSettings::deserialize(deserializer)?;
    validate_capital_pitch(&settings).map_err(serde::de::Error::custom)?;
    Ok(Some(settings))
}

fn present_punctuation<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<PunctuationTables>, D::Error> {
    let tables = PunctuationTables::deserialize(deserializer)?;
    tables.validate().map_err(serde::de::Error::custom)?;
    Ok(Some(tables))
}

fn present_audio<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<AudioOutputSettings>, D::Error> {
    AudioOutputSettings::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationWire {
    engine_id: String,
    origin: String,
    #[serde(deserialize_with = "required_nullable")]
    source: Option<OsString>,
    #[serde(deserialize_with = "required_nullable")]
    override_source: Option<OsString>,
    enabled: bool,
    #[serde(deserialize_with = "required_nullable")]
    unavailable: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    helper: Option<HelperWire>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperWire {
    program: OsString,
    arguments: Vec<OsString>,
    startup_ms: u64,
    request_ms: u64,
    synthesis_idle_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutingWire {
    #[serde(deserialize_with = "required_nullable")]
    preferred_engine_ids: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    fallback_engine_ids: Option<Vec<String>>,
    disabled_engine_ids: Vec<String>,
    automatic_engine_ids: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedWire {
    path: OsString,
    // Preserve original JSON bytes, including whitespace, for its pinned hash.
    generation: String,
    piper_override: bool,
    flite_override: bool,
}

impl From<&LaunchSnapshot> for SnapshotWire {
    fn from(snapshot: &LaunchSnapshot) -> Self {
        let resolved = &snapshot.resolved;
        Self {
            schema: snapshot.schema,
            speech: (snapshot.schema >= 2).then_some(ChunkSpeechWire {
                max_chunk_words: resolved.speech.max_chunk_words,
            }),
            speech_defaults: (snapshot.schema >= 3).then(|| resolved.speech.defaults.clone()),
            capital_pitch: (snapshot.schema >= 4).then(|| resolved.speech.capital_pitch.clone()),
            audio: (snapshot.schema >= 5).then_some(resolved.audio),
            punctuation: (snapshot.schema >= 6).then(|| resolved.speech.punctuation.clone()),
            platform: std::env::consts::OS.into(),
            activation_id: snapshot.activation_id.clone(),
            registrations: resolved
                .registrations()
                .map(|entry| RegistrationWire {
                    engine_id: entry.engine_id.clone(),
                    origin: match entry.origin {
                        EngineOrigin::InProcess => "in_process",
                        EngineOrigin::ShippedHelper => "shipped_helper",
                        EngineOrigin::ExternalHelper => "external_helper",
                    }
                    .into(),
                    source: entry.source.clone().map(PathBuf::into_os_string),
                    override_source: entry.override_source.clone().map(PathBuf::into_os_string),
                    enabled: entry.enabled,
                    unavailable: entry.unavailable.clone(),
                    helper: entry.helper.as_ref().map(|helper| HelperWire {
                        program: helper.program.clone().into_os_string(),
                        arguments: helper.arguments.clone(),
                        startup_ms: helper
                            .startup_timeout
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        request_ms: helper
                            .request_timeout
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        synthesis_idle_ms: helper
                            .synthesis_idle_timeout
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                    }),
                })
                .collect(),
            routing: RoutingWire {
                preferred_engine_ids: resolved.routing.preferred_engine_ids.clone(),
                fallback_engine_ids: resolved.routing.fallback_engine_ids.clone(),
                disabled_engine_ids: resolved.routing.disabled_engine_ids.clone(),
                automatic_engine_ids: resolved.routing.automatic_engine_ids.clone(),
            },
            environment: resolved
                .environment
                .variables()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            root: resolved.root.clone().map(PathBuf::into_os_string),
            managed: snapshot.managed.as_ref().map(|managed| ManagedWire {
                path: managed.path.clone().into_os_string(),
                // RuntimeLibrary has already validated these exact UTF-8 bytes.
                generation: String::from_utf8(managed.library.source_bytes().to_vec())
                    .expect("validated library JSON"),
                piper_override: managed.overrides.piper,
                flite_override: managed.overrides.flite,
            }),
            requested: snapshot.requested.clone(),
            piper_selected: snapshot.piper_selected,
        }
    }
}

impl SnapshotWire {
    fn restore(self) -> Result<LaunchSnapshot> {
        let mut speech = match (self.schema, self.speech, self.speech_defaults) {
            (1, None, None) => SpeechConfiguration::default(),
            (2, Some(speech), None) => SpeechConfiguration {
                max_chunk_words: speech.max_chunk_words,
                ..SpeechConfiguration::default()
            },
            (3..=7, Some(speech), Some(defaults)) => SpeechConfiguration {
                max_chunk_words: speech.max_chunk_words,
                defaults,
                ..SpeechConfiguration::default()
            },
            _ => return Err(invalid("unsupported or incomplete launch snapshot schema")),
        };
        speech.capital_pitch = match (self.schema, self.capital_pitch) {
            (1..=3, None) => CapitalPitchSettings::default(),
            (4..=7, Some(settings)) => settings,
            _ => return Err(invalid("unsupported or incomplete capital pitch settings")),
        };
        speech.punctuation = match (self.schema, self.punctuation) {
            (1..=5, None) => PunctuationTables::legacy(),
            (6, Some(tables)) if tables.profiles.is_empty() => tables,
            (7, Some(tables)) => tables,
            _ => return Err(invalid("unsupported or incomplete punctuation tables")),
        };
        let audio = match (self.schema, self.audio) {
            (1..=4, None) => AudioOutputSettings::default(),
            (5..=7, Some(settings)) => settings,
            _ => return Err(invalid("unsupported or incomplete audio settings")),
        };
        require(
            self.platform == std::env::consts::OS,
            "launch snapshot platform differs",
        )?;
        require(
            valid_uuid(&self.activation_id),
            "invalid activation identity",
        )?;
        require(!self.requested.contains('\0'), "invalid requested engine")?;
        validate_environment(&self.environment)?;
        let environment = LaunchEnvironment::from_variables(self.environment);
        let root = self.root.map(PathBuf::from);
        validate_source(root.as_deref())?;
        let routing = self.routing.restore()?;
        require(
            self.registrations.len() <= shipped::ENGINES.len() + MAX_MANIFESTS,
            "too many snapshot registrations",
        )?;
        let mut registrations = BTreeMap::new();
        let mut external_count = 0;
        for entry in self.registrations {
            let registration = entry.restore(&environment, &routing)?;
            if registration.origin == EngineOrigin::ExternalHelper {
                external_count += 1;
            }
            require(
                registrations
                    .insert(registration.engine_id.clone(), registration)
                    .is_none(),
                "duplicate snapshot registration",
            )?;
        }
        require(
            external_count <= MAX_MANIFESTS,
            "too many external registrations",
        )?;
        require(
            shipped::ENGINES
                .iter()
                .all(|entry| registrations.contains_key(entry.id)),
            "incomplete shipped registration set",
        )?;
        require(
            speech
                .capital_pitch
                .engines
                .keys()
                .all(|id| registrations.contains_key(id)),
            "capital pitch engine is not registered",
        )?;
        let managed = self.managed.map(ManagedWire::restore).transpose()?;
        if let Some(managed) = &managed {
            validate_managed_invocations(managed, &registrations)?;
        }
        Ok(LaunchSnapshot {
            schema: self.schema,
            activation_id: self.activation_id,
            resolved: ResolvedConfiguration {
                registrations,
                routing,
                speech,
                audio,
                environment,
                root,
                // Input diagnostics were emitted by the preparing owner. A
                // worker consumes the retained valid set without rediscovery.
                diagnostics: Vec::new(),
            },
            managed,
            requested: self.requested,
            piper_selected: self.piper_selected,
        })
    }
}

impl RegistrationWire {
    fn restore(
        self,
        environment: &LaunchEnvironment,
        routing: &LocalRoutingPolicy,
    ) -> Result<ResolvedRegistration> {
        validate_reference(&self.engine_id, "snapshot.engine_id")?;
        let origin = match (shipped::definition(&self.engine_id), self.origin.as_str()) {
            (Some(definition), "in_process") if definition.in_process => EngineOrigin::InProcess,
            (Some(definition), "shipped_helper") if !definition.in_process => {
                EngineOrigin::ShippedHelper
            }
            (None, "external_helper") => {
                validate_external_id(&self.engine_id)?;
                EngineOrigin::ExternalHelper
            }
            _ => return Err(invalid("snapshot registration origin differs")),
        };
        let source = self.source.map(PathBuf::from);
        let override_source = self.override_source.map(PathBuf::from);
        validate_source(source.as_deref())?;
        validate_source(override_source.as_deref())?;
        require(
            source.is_some() == (origin == EngineOrigin::ExternalHelper),
            "snapshot registration source differs",
        )?;
        require(
            !routing.disabled_engine_ids.contains(&self.engine_id) || !self.enabled,
            "snapshot enables excluded engine",
        )?;
        require(
            self.enabled || self.unavailable.is_some(),
            "disabled snapshot engine is available",
        )?;
        require(
            self.unavailable
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 4096),
            "invalid snapshot availability",
        )?;
        let helper = self
            .helper
            .map(|helper| helper.restore(&self.engine_id, origin, environment))
            .transpose()?;
        require(
            origin != EngineOrigin::InProcess || helper.is_none(),
            "in-process snapshot has helper launch data",
        )?;
        require(
            origin != EngineOrigin::ExternalHelper || helper.is_some(),
            "external snapshot lacks helper launch data",
        )?;
        require(
            origin != EngineOrigin::ShippedHelper || helper.is_some() || self.unavailable.is_some(),
            "available helper lacks launch data",
        )?;
        if origin == EngineOrigin::InProcess {
            let native_available = self.engine_id == "espeak"
                || (self.engine_id == "winrt" && cfg!(windows))
                || (self.engine_id == "macos" && cfg!(target_os = "macos"));
            require(
                native_available || self.unavailable.is_some(),
                "snapshot enables unsupported native engine",
            )?;
        }
        Ok(ResolvedRegistration {
            engine_id: self.engine_id,
            origin,
            source,
            override_source,
            enabled: self.enabled,
            unavailable: self.unavailable,
            helper,
        })
    }
}

impl HelperWire {
    fn restore(
        self,
        id: &str,
        origin: EngineOrigin,
        environment: &LaunchEnvironment,
    ) -> Result<HelperEngineConfig> {
        require(
            !self.program.is_empty() && native_without_nul(&self.program),
            "invalid snapshot helper program",
        )?;
        require(
            self.arguments.iter().all(|arg| native_without_nul(arg)),
            "invalid snapshot helper argument",
        )?;
        if origin == EngineOrigin::ExternalHelper {
            Platform::native().validate_path(
                self.program
                    .to_str()
                    .ok_or_else(|| invalid("invalid external program encoding"))?,
                "snapshot.program",
            )?;
            let arguments = self
                .arguments
                .iter()
                .map(|arg| {
                    arg.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("invalid external argument encoding"))
                })
                .collect::<Result<Vec<_>>>()?;
            validate_arguments(&arguments, "snapshot.arguments")?;
        }
        let timeouts = Timeouts::parse(
            serde_json::json!({
                "startup_ms": self.startup_ms,
                "request_ms": self.request_ms,
                "synthesis_idle_ms": self.synthesis_idle_ms,
            }),
            "snapshot.timeouts",
        )?;
        Ok(HelperEngineConfig {
            engine_id: id.into(),
            program: self.program.into(),
            arguments: self.arguments,
            environment: environment.clone(),
            startup_timeout: Duration::from_millis(timeouts.startup_ms.unwrap()),
            request_timeout: Duration::from_millis(timeouts.request_ms.unwrap()),
            synthesis_idle_timeout: Duration::from_millis(timeouts.synthesis_idle_ms.unwrap()),
        })
    }
}

impl RoutingWire {
    fn restore(self) -> Result<LocalRoutingPolicy> {
        let mut policy = serde_json::json!({
            "disabled_engine_ids": self.disabled_engine_ids,
            "automatic_engine_ids": self.automatic_engine_ids,
        });
        if let Some(preferred) = self.preferred_engine_ids {
            policy["preferred_engine_ids"] = serde_json::json!(preferred);
        }
        if let Some(fallback) = self.fallback_engine_ids {
            policy["fallback_engine_ids"] = serde_json::json!(fallback);
        }
        LocalRoutingPolicy::parse(policy)
    }
}

impl ManagedWire {
    fn restore(self) -> Result<ManagedLaunch> {
        let path = PathBuf::from(self.path);
        validate_source(Some(&path))?;
        let library = RuntimeLibrary::parse(
            self.generation.as_bytes(),
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )
        .map_err(|_| invalid("invalid managed generation in snapshot"))?;
        Ok(ManagedLaunch {
            path,
            library,
            overrides: ProviderOverrides {
                piper: self.piper_override,
                flite: self.flite_override,
            },
        })
    }
}

fn validate_managed_invocations(
    managed: &ManagedLaunch,
    registrations: &BTreeMap<String, ResolvedRegistration>,
) -> Result<()> {
    let document = managed.library.document();
    let eligibility =
        crate::voice_library::VoiceEligibility::from_library(&managed.library, managed.overrides);
    for (id, active) in [
        (
            "piper",
            document.piper.is_some() && !managed.overrides.piper,
        ),
        (
            "flite",
            document.flite.is_some() && !managed.overrides.flite,
        ),
        ("mbrola", document.mbrola.is_some()),
        ("rhvoice", document.rhvoice.is_some()),
    ] {
        let entry = &registrations[id];
        if active {
            if let Some(helper) = &entry.helper {
                let expected: Vec<OsString> = vec![
                    "--voice-library".into(),
                    managed.path.clone().into_os_string(),
                    "--voice-library-sha256".into(),
                    managed.library.sha256().into(),
                ];
                require(
                    helper.arguments == expected,
                    "snapshot replaced managed invocation",
                )?;
            }
        }
        require(
            !eligibility.excludes_provider(id) || entry.unavailable.is_some(),
            "snapshot enables excluded provider",
        )?;
    }
    Ok(())
}

pub(super) fn validate_environment(variables: &[(OsString, OsString)]) -> Result<()> {
    for (name, value) in variables {
        require(
            !name.is_empty() && native_without_nul(name) && native_without_nul(value),
            "invalid snapshot environment",
        )?;
        // Capture omits Windows' hidden drive-directory entries. A complete
        // record must already contain launch settings, never those entries.
        require(
            !name.as_encoded_bytes().contains(&b'='),
            "invalid snapshot environment name",
        )?;
    }
    // Use the same native key comparison as an actual launch, including Windows
    // case folding, without lossy conversion of non-Unicode variable names.
    let mut command = std::process::Command::new("unused-snapshot-validation");
    command
        .env_clear()
        .envs(variables.iter().map(|(name, value)| (name, value)));
    require(
        command.get_envs().count() == variables.len(),
        "duplicate snapshot environment name",
    )
}

fn native_without_nul(value: &OsStr) -> bool {
    !value.as_encoded_bytes().contains(&0)
}
fn validate_source(path: Option<&std::path::Path>) -> Result<()> {
    require(
        path.is_none_or(|path| path.is_absolute() && native_without_nul(path.as_os_str())),
        "invalid snapshot source path",
    )
}
fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, byte)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}
fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}
fn invalid(reason: &'static str) -> ConfigurationError {
    ConfigurationError::new("launch snapshot", reason)
}
fn require(condition: bool, reason: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(reason))
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
