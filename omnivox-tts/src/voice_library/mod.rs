//! Validated voice-library metadata, without filesystem mutation or native loading.
//!
//! Parsing establishes structural consistency only. Asset hashes, ownership,
//! native compatibility and activation must be checked by the management service.
//! Keep the original bytes for generation hashing; never hash a reserialization.

use std::collections::HashSet;
use std::io::Read;

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::contracts::PhysicalVoiceId;

mod index;
mod runtime;

pub use index::{
    CatalogueReference, FileRole, IndexDocument, IndexedFile, IndexedVoice, LibraryIndex,
    NativeValidation, Ownership, PackageRevision, Provider,
};
pub use runtime::{
    ActivePointer, FliteLibrary, FliteVoice, PiperLibrary, PiperModel, PiperVoice, RuntimeDocument,
    RuntimeLibrary,
};

pub const MAX_RUNTIME_BYTES: usize = 1024 * 1024;
pub const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIPER_MODELS: usize = 64;
pub const MAX_FLITE_FILES: usize = 64;
pub const MAX_PROJECTED_VOICES: usize = 256;
pub const MAX_DISABLED_VOICES: usize = 4096;
pub const MAX_PACKAGE_REVISIONS: usize = 1024;
pub const MAX_INDEX_VOICES: usize = 4096;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("invalid voice library: {0}")]
    Invalid(&'static str),
    #[error("invalid voice-library JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not read voice library: {0}")]
    Io(#[from] std::io::Error),
}

/// The speech host's path rules, which may differ from the manager's OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPlatform {
    Windows,
    Posix,
}

/// Immutable identity independent of filenames, package revisions and labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ModelIdentity {
    Catalogue { catalogue_key: String },
    Import { import_id: String },
}

impl ModelIdentity {
    fn validate(&self) -> Result<(), LibraryError> {
        match self {
            Self::Catalogue { catalogue_key } => {
                let bytes = catalogue_key.as_bytes();
                require(
                    !bytes.is_empty() && bytes.len() <= 96,
                    "invalid catalogue key",
                )?;
                require(
                    bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit(),
                    "invalid catalogue key prefix",
                )?;
                require(
                    bytes.iter().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_'
                    }),
                    "invalid catalogue key",
                )
            }
            Self::Import { import_id } => uuid(import_id),
        }
    }

    fn sort_key(&self) -> (&str, &str) {
        match self {
            Self::Catalogue { catalogue_key } => ("c", catalogue_key),
            Self::Import { import_id } => ("i", import_id),
        }
    }

    /// Construct the canonical identity for a native speaker index.
    pub fn piper_voice_id(&self, speaker_index: u32) -> Result<String, LibraryError> {
        self.validate()?;
        speaker(speaker_index)?;
        let (namespace, key) = self.sort_key();
        Ok(format!("piper:v1/{namespace}/{key}/{speaker_index}"))
    }

    fn validate_binding(&self, id: &str, index: u32) -> Result<(), LibraryError> {
        let canonical = self.piper_voice_id(index)?;
        if id == canonical {
            return Ok(());
        }
        require(
            matches!(self, Self::Import { .. }) && index == 0 && legacy_piper_id(id),
            "Piper voice identity does not match its model and speaker",
        )
    }
}

/// A declared asset; parsing this record never opens the path or verifies bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl AssetFile {
    fn validate(&self, host: HostPlatform) -> Result<(), LibraryError> {
        native_path(&self.path, host)?;
        require(self.bytes > 0, "asset size must be positive")?;
        sha256(&self.sha256)
    }
}

fn require(condition: bool, reason: &'static str) -> Result<(), LibraryError> {
    if condition {
        Ok(())
    } else {
        Err(LibraryError::Invalid(reason))
    }
}

fn text(value: &str, max: usize) -> Result<(), LibraryError> {
    require(
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control),
        "empty, oversized or control-containing string",
    )
}

fn uuid(value: &str) -> Result<(), LibraryError> {
    require(
        value.len() == 36
            && value.bytes().enumerate().all(|(i, byte)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                }
            }),
        "UUID must have canonical lowercase spelling",
    )
}

fn sha256(value: &str) -> Result<(), LibraryError> {
    require(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "SHA-256 must be 64 lowercase hexadecimal digits",
    )
}

fn speaker(value: u32) -> Result<(), LibraryError> {
    require(
        value <= i32::MAX as u32,
        "speaker index exceeds native range",
    )
}

fn legacy_piper_id(value: &str) -> bool {
    value.strip_prefix("piper:").is_some_and(|stem| {
        !stem.is_empty()
            && !stem.starts_with("v1/")
            && !stem.contains(['/', '\\'])
            && text(value, 16 * 1024).is_ok()
    })
}

fn native_path(value: &str, host: HostPlatform) -> Result<(), LibraryError> {
    text(value, 4096)?;
    let absolute = match host {
        HostPlatform::Posix => value.starts_with('/'),
        HostPlatform::Windows => {
            let bytes = value.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            let unc = value.strip_prefix("\\\\").is_some_and(|tail| {
                let mut parts = tail.split('\\');
                let server = parts.next().unwrap_or_default();
                let share = parts.next().unwrap_or_default();
                !server.is_empty() && !matches!(server, "." | "?") && !share.is_empty()
            });
            drive || unc
        }
    };
    require(absolute, "asset path is not absolute on the speech host")
}

fn voice_metadata(name: &str, language: &Option<String>) -> Result<(), LibraryError> {
    text(name, 256)?;
    if let Some(language) = language {
        text(language, 64)?;
    }
    Ok(())
}

// Native/legacy voice IDs retain their spelling, including Unicode. New
// library identities have the stricter grammar above.
fn physical_id(engine: &str, voice: &str) -> Result<(), LibraryError> {
    text(engine, 128)?;
    require(engine.is_ascii(), "engine ID must be ASCII")?;
    text(voice, 16 * 1024)
}

fn exclusions(values: &[PhysicalVoiceId]) -> Result<(), LibraryError> {
    require(
        values.len() <= MAX_DISABLED_VOICES,
        "too many disabled voices",
    )?;
    let mut previous = None;
    for value in values {
        physical_id(&value.engine_id, &value.voice_id)?;
        let key = (value.engine_id.as_str(), value.voice_id.as_str());
        require(
            previous.is_none_or(|old| old < key),
            "disabled voices must be sorted and unique",
        )?;
        previous = Some(key);
    }
    Ok(())
}

fn disabled_set(values: &[PhysicalVoiceId]) -> HashSet<(&str, &str)> {
    values
        .iter()
        .map(|v| (v.engine_id.as_str(), v.voice_id.as_str()))
        .collect()
}

fn deserialize_exclusions<'de, D>(deserializer: D) -> Result<Vec<PhysicalVoiceId>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictPhysicalId {
        engine_id: String,
        voice_id: String,
    }
    Ok(Vec::<StrictPhysicalId>::deserialize(deserializer)?
        .into_iter()
        .map(|v| PhysicalVoiceId::new(v.engine_id, v.voice_id))
        .collect())
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn decode<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, LibraryError> {
    require(bytes.len() <= limit, "JSON input exceeds byte limit")?;
    serde_json::from_slice::<crate::control::DuplicateFreeJson>(bytes)?;
    Ok(serde_json::from_slice(bytes)?)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, LibraryError> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    require(bytes.len() <= limit, "JSON input exceeds byte limit")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
