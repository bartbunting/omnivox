//! Version-1 local engine configuration. Parsing never launches code or writes files.
//!
//! Registration, launch overrides and local routing permissions remain distinct
//! from live descriptors and managed asset verification.

mod environment;
mod files;
mod json;
mod paths;
mod resolved;
mod selection;
pub mod shipped;

use std::collections::{BTreeMap, BTreeSet};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

pub use environment::LaunchEnvironment;
pub use files::{ConfigurationRoot, LoadedConfiguration, ManifestRegistration};
pub use paths::Platform;
pub use resolved::{
    EngineOrigin, ResolvedConfiguration, ResolvedRegistration, RuntimeInputs, RuntimeInvocation,
};
pub use selection::EngineSelectionPermissions;

pub const MAX_CONFIG_BYTES: usize = 128 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_MANIFESTS: usize = 32;
pub const MAX_MANIFEST_TOTAL_BYTES: usize = 1024 * 1024;
pub const MAX_JSON_DEPTH: usize = 16;
pub const MAX_ENGINE_IDS: usize = 64;

/// Values and parser errors are deliberately omitted: arguments may be private.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field}: {reason}")]
pub struct ConfigurationError {
    pub field: String,
    pub reason: &'static str,
}

impl ConfigurationError {
    fn new(field: impl Into<String>, reason: &'static str) -> Self {
        Self {
            field: field.into(),
            reason,
        }
    }
}

type Result<T> = std::result::Result<T, ConfigurationError>;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRoutingPolicy {
    pub preferred_engine_ids: Option<Vec<String>>,
    pub fallback_engine_ids: Option<Vec<String>>,
    pub disabled_engine_ids: Vec<String>,
    pub automatic_engine_ids: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeouts {
    pub startup_ms: Option<u64>,
    pub request_ms: Option<u64>,
    pub synthesis_idle_ms: Option<u64>,
}

impl Timeouts {
    fn parse(value: Value, field: &str) -> Result<Self> {
        let mut object = Object::new(value, field)?;
        let result = Self {
            startup_ms: object.optional("startup_ms")?,
            request_ms: object.optional("request_ms")?,
            synthesis_idle_ms: object.optional("synthesis_idle_ms")?,
        };
        object.finish()?;
        for (name, value, maximum) in [
            ("startup_ms", result.startup_ms, 120_000),
            ("request_ms", result.request_ms, 30_000),
            ("synthesis_idle_ms", result.synthesis_idle_ms, 300_000),
        ] {
            if value.is_some_and(|value| !(100..=maximum).contains(&value)) {
                return Err(ConfigurationError::new(
                    format!("{field}.{name}"),
                    "timeout outside allowed range",
                ));
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineOverride {
    pub enabled: Option<bool>,
    pub program: Option<String>,
    pub arguments: Option<Vec<String>>,
    pub timeouts: Option<Timeouts>,
}

impl EngineOverride {
    fn parse(value: Value, platform: Platform) -> Result<Self> {
        let mut object = Object::new(value, "engine_overrides.entry")?;
        let result = Self {
            enabled: object.optional("enabled")?,
            program: object.optional("program")?,
            arguments: object.optional("arguments")?,
            timeouts: object.timeouts()?,
        };
        object.finish()?;
        if let Some(program) = &result.program {
            platform.validate_path(program, "engine_overrides.entry.program")?;
        }
        if let Some(arguments) = &result.arguments {
            validate_arguments(arguments, "engine_overrides.entry.arguments")?;
        }
        Ok(result)
    }

    pub fn has_launch_fields(&self) -> bool {
        self.program.is_some() || self.arguments.is_some() || self.timeouts.is_some()
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub routing: LocalRoutingPolicy,
    pub engine_overrides: BTreeMap<String, EngineOverride>,
}

impl Configuration {
    pub fn parse(bytes: &[u8], platform: Platform) -> Result<Self> {
        let mut object = Object::new(json::parse(bytes, MAX_CONFIG_BYTES)?, "config.json")?;
        object.schema()?;
        let routing = object
            .take("routing")
            .map(LocalRoutingPolicy::parse)
            .transpose()?
            .unwrap_or_default();
        let mut engine_overrides = BTreeMap::new();
        if let Some(value) = object.take("engine_overrides") {
            let entries = Object::new(value, "engine_overrides")?;
            if entries.fields.len() > MAX_ENGINE_IDS {
                return Err(ConfigurationError::new(
                    "engine_overrides",
                    "too many entries",
                ));
            }
            for (id, value) in entries.fields {
                validate_reference(&id, "engine_overrides")?;
                engine_overrides.insert(id, EngineOverride::parse(value, platform)?);
            }
        }
        object.finish()?;
        Ok(Self {
            routing,
            engine_overrides,
        })
    }

    /// Resolve references only after conflicts and whole-set failures are known.
    /// Managed invocation ownership is validated by the launch resolver.
    pub fn validate_overrides(&self, external_ids: &BTreeSet<String>) -> Result<()> {
        for (id, value) in &self.engine_overrides {
            match shipped::definition(id) {
                Some(definition) if definition.in_process && value.has_launch_fields() => {
                    return Err(ConfigurationError::new(
                        "engine_overrides.entry",
                        "in-process engines accept only enabled",
                    ));
                }
                Some(_) => (),
                None if external_ids.contains(id) => (),
                None => {
                    return Err(ConfigurationError::new(
                        "engine_overrides",
                        "override requires a valid registered engine",
                    ))
                }
            }
        }
        Ok(())
    }
}

impl LocalRoutingPolicy {
    fn parse(value: Value) -> Result<Self> {
        let mut object = Object::new(value, "routing")?;
        let result = Self {
            preferred_engine_ids: object.ids("preferred_engine_ids")?,
            fallback_engine_ids: object.ids("fallback_engine_ids")?,
            disabled_engine_ids: object.ids("disabled_engine_ids")?.unwrap_or_default(),
            automatic_engine_ids: object.ids("automatic_engine_ids")?.unwrap_or_default(),
        };
        object.finish()?;
        Ok(result)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperManifest {
    pub engine_id: String,
    pub enabled: bool,
    pub program: String,
    pub arguments: Vec<String>,
    pub timeouts: Timeouts,
}

impl HelperManifest {
    pub fn parse(bytes: &[u8], platform: Platform) -> Result<Self> {
        let mut object = Object::new(json::parse(bytes, MAX_MANIFEST_BYTES)?, "manifest")?;
        object.schema()?;
        let result = Self {
            engine_id: object.required("engine_id")?,
            enabled: object.optional("enabled")?.unwrap_or(true),
            program: object.required("program")?,
            arguments: object.optional("arguments")?.unwrap_or_default(),
            timeouts: object.timeouts()?.unwrap_or_default(),
        };
        object.finish()?;
        validate_external_id(&result.engine_id)?;
        platform.validate_path(&result.program, "manifest.program")?;
        validate_arguments(&result.arguments, "manifest.arguments")?;
        Ok(result)
    }
}

fn validate_external_id(id: &str) -> Result<()> {
    let mut segments = id.split(['.', '_', '-']);
    let valid = id.len() <= 128
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && segments.all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if !valid {
        return Err(ConfigurationError::new(
            "manifest.engine_id",
            "invalid external engine ID",
        ));
    }
    Ok(())
}

fn validate_reference(id: &str, field: &str) -> Result<()> {
    // Existing routing IDs permit more than the grammar for new registrations.
    // Compatibility aliases remain confined to CLI/environment selection.
    if id.is_empty()
        || id.len() > 128
        || id.chars().any(|c| c.is_whitespace() || c.is_control())
        || shipped::ALIASES.contains(&id)
    {
        return Err(ConfigurationError::new(
            field,
            "invalid canonical engine ID",
        ));
    }
    Ok(())
}

fn validate_arguments(arguments: &[String], field: &str) -> Result<()> {
    if arguments.len() > 64
        || arguments
            .iter()
            .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        || arguments.iter().map(String::len).sum::<usize>() > 16 * 1024
    {
        return Err(ConfigurationError::new(
            field,
            "invalid argument vector or size",
        ));
    }
    Ok(())
}

struct Object {
    fields: Map<String, Value>,
    field: String,
}

impl Object {
    fn new(value: Value, field: &str) -> Result<Self> {
        match value {
            Value::Object(fields) => Ok(Self {
                fields,
                field: field.into(),
            }),
            _ => Err(ConfigurationError::new(field, "expected object")),
        }
    }

    fn take(&mut self, key: &str) -> Option<Value> {
        self.fields.remove(key)
    }

    fn optional<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        self.take(key)
            .map(|value| {
                serde_json::from_value(value).map_err(|_| {
                    ConfigurationError::new(
                        format!("{}.{key}", self.field),
                        "invalid type or value",
                    )
                })
            })
            .transpose()
    }

    fn required<T: DeserializeOwned>(&mut self, key: &str) -> Result<T> {
        self.optional(key)?.ok_or_else(|| {
            ConfigurationError::new(format!("{}.{key}", self.field), "required field missing")
        })
    }

    fn schema(&mut self) -> Result<()> {
        if self.required::<u64>("schema")? != 1 {
            return Err(ConfigurationError::new(
                format!("{}.schema", self.field),
                "unsupported schema",
            ));
        }
        Ok(())
    }

    fn timeouts(&mut self) -> Result<Option<Timeouts>> {
        self.take("timeouts")
            .map(|value| Timeouts::parse(value, &format!("{}.timeouts", self.field)))
            .transpose()
    }

    fn ids(&mut self, key: &str) -> Result<Option<Vec<String>>> {
        let values: Option<Vec<String>> = self.optional(key)?;
        if let Some(values) = &values {
            let field = format!("{}.{key}", self.field);
            if values.len() > MAX_ENGINE_IDS {
                return Err(ConfigurationError::new(field, "too many engine IDs"));
            }
            let mut seen = BTreeSet::new();
            for id in values {
                validate_reference(id, &field)?;
                if !seen.insert(id) {
                    return Err(ConfigurationError::new(field, "duplicate engine ID"));
                }
            }
        }
        Ok(values)
    }

    fn finish(self) -> Result<()> {
        if self.fields.is_empty() {
            Ok(())
        } else {
            Err(ConfigurationError::new(self.field, "unknown field"))
        }
    }
}

#[cfg(test)]
mod tests;
