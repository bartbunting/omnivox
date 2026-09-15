use std::collections::HashMap;

use super::*;

/// Desired installation/enablement records. Use `LibraryIndex::parse` to validate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexDocument {
    pub schema_version: u32,
    pub target_id: String,
    pub profile_id: String,
    pub revision_id: String,
    pub packages: Vec<PackageRevision>,
    pub voices: Vec<IndexedVoice>,
    #[serde(deserialize_with = "deserialize_exclusions")]
    pub disabled_physical_ids: Vec<PhysicalVoiceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Piper,
    Flite,
}

impl Provider {
    fn engine_id(self) -> &'static str {
        match self {
            Self::Piper => "piper",
            Self::Flite => "flite",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Managed,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageRevision {
    pub package_id: String,
    pub revision_id: String,
    pub provider: Provider,
    pub ownership: Ownership,
    pub identity: ModelIdentity,
    pub files: Vec<IndexedFile>,
    #[serde(deserialize_with = "required_nullable")]
    pub validation: Option<NativeValidation>,
    #[serde(deserialize_with = "required_nullable")]
    pub catalogue: Option<CatalogueReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileRole {
    Model,
    Config,
    Voice,
}

// Deliberately not flattened: serde flattening would weaken strict field checks.
// Field order also defines the contract's file-set hash input serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedFile {
    pub role: FileRole,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeValidation {
    pub validator_version: String,
    pub target_id: String,
    pub validated_at: String,
    pub file_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueReference {
    pub revision: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexedVoice {
    pub physical_id: String,
    pub engine_id: String,
    pub display_name: String,
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
    pub enabled: bool,
    #[serde(deserialize_with = "required_nullable")]
    pub package_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub revision_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub speaker_index: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_string"
    )]
    pub legacy_physical_id: Option<String>,
}

fn present_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

/// Structurally consistent desired state, which may still need native validation.
#[derive(Debug, Clone)]
pub struct LibraryIndex {
    document: IndexDocument,
    source_bytes: Vec<u8>,
}

impl LibraryIndex {
    pub fn parse(bytes: &[u8], host: HostPlatform) -> Result<Self, LibraryError> {
        let document: IndexDocument = decode(bytes, MAX_INDEX_BYTES)?;
        document.validate(host)?;
        Ok(Self {
            document,
            source_bytes: bytes.to_vec(),
        })
    }

    pub fn read(reader: impl Read, host: HostPlatform) -> Result<Self, LibraryError> {
        Self::parse(&read_bounded(reader, MAX_INDEX_BYTES)?, host)
    }

    pub fn document(&self) -> &IndexDocument {
        &self.document
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }
}

impl IndexDocument {
    fn validate(&self, host: HostPlatform) -> Result<(), LibraryError> {
        require(self.schema_version == 1, "unsupported index schema")?;
        uuid(&self.target_id)?;
        uuid(&self.profile_id)?;
        uuid(&self.revision_id)?;
        exclusions(&self.disabled_physical_ids)?;
        require(
            self.packages.len() <= MAX_PACKAGE_REVISIONS,
            "too many package revisions",
        )?;
        require(
            self.voices.len() <= MAX_INDEX_VOICES,
            "too many indexed voices",
        )?;
        let mut packages = HashMap::new();
        let mut identities = HashMap::new();
        let mut package_identities = HashMap::new();
        for package in &self.packages {
            package.validate(host)?;
            let key = (package.package_id.as_str(), package.revision_id.as_str());
            require(
                packages.insert(key, package).is_none(),
                "duplicate package revision",
            )?;
            require(
                identities
                    .insert(&package.identity, &package.package_id)
                    .is_none_or(|id| id == &package.package_id),
                "model identity belongs to more than one package",
            )?;
            require(
                package_identities
                    .insert(&package.package_id, (&package.identity, package.provider))
                    .is_none_or(|binding| binding == (&package.identity, package.provider)),
                "package changed its stable identity or provider",
            )?;
        }
        let disabled = disabled_set(&self.disabled_physical_ids);
        let mut ids = HashSet::new();
        let mut bindings = HashSet::new();
        for voice in &self.voices {
            physical_id(&voice.engine_id, &voice.physical_id)?;
            voice_metadata(&voice.display_name, &voice.language)?;
            let id = (voice.engine_id.as_str(), voice.physical_id.as_str());
            require(ids.insert(id), "duplicate indexed physical voice")?;
            require(
                voice.enabled != disabled.contains(&id),
                "enabled state contradicts disabled physical IDs",
            )?;
            match (&voice.package_id, &voice.revision_id) {
                (Some(package_id), Some(revision_id)) => {
                    let package = packages
                        .get(&(package_id.as_str(), revision_id.as_str()))
                        .ok_or(LibraryError::Invalid(
                            "voice references a missing package revision",
                        ))?;
                    require(
                        voice.engine_id == package.provider.engine_id(),
                        "voice/provider mismatch",
                    )?;
                    match package.provider {
                        Provider::Piper => {
                            let index = voice.speaker_index.ok_or(LibraryError::Invalid(
                                "Piper voice needs a speaker index",
                            ))?;
                            package
                                .identity
                                .validate_binding(&voice.physical_id, index)?;
                            require(
                                bindings.insert((&package.identity, index)),
                                "multiple IDs refer to the same model speaker",
                            )?;
                            let is_legacy =
                                voice.physical_id != package.identity.piper_voice_id(index)?;
                            require(
                                if is_legacy {
                                    voice.legacy_physical_id.as_ref() == Some(&voice.physical_id)
                                } else {
                                    voice.legacy_physical_id.is_none()
                                },
                                "legacy adoption must be explicit and match the physical ID",
                            )?;
                        }
                        Provider::Flite => {
                            runtime::flite_id(&voice.physical_id)?;
                            require(
                                voice.speaker_index.is_none() && voice.legacy_physical_id.is_none(),
                                "Flite voice cannot specify a speaker or legacy Piper ID",
                            )?;
                            require(
                                bindings.insert((&package.identity, 0)),
                                "Flite package exposes more than one voice",
                            )?;
                        }
                    }
                }
                (None, None) => {
                    require(
                        voice.speaker_index.is_none() && voice.legacy_physical_id.is_none(),
                        "unpackaged voice cannot specify a speaker or legacy adoption",
                    )?;
                    require(
                        voice.engine_id != "piper"
                            && (voice.engine_id != "flite" || voice.physical_id == "cmu_us_slt"),
                        "external voice needs a package revision",
                    )?;
                }
                _ => return Err(LibraryError::Invalid("partial package reference")),
            }
        }
        Ok(())
    }
}

impl PackageRevision {
    fn validate(&self, host: HostPlatform) -> Result<(), LibraryError> {
        uuid(&self.package_id)?;
        uuid(&self.revision_id)?;
        self.identity.validate()?;
        if self.ownership == Ownership::Managed {
            require(
                self.catalogue.is_some(),
                "managed package needs catalogue provenance",
            )?;
        }
        if let Some(catalogue) = &self.catalogue {
            for value in [&catalogue.revision, &catalogue.entry_id] {
                text(value, 128)?;
                require(value.is_ascii(), "catalogue identifiers must be ASCII")?;
            }
        }
        let expected: &[FileRole] = match self.provider {
            Provider::Piper => &[FileRole::Model, FileRole::Config],
            Provider::Flite => &[FileRole::Voice],
        };
        require(
            self.files.len() == expected.len(),
            "wrong number of package files",
        )?;
        let mut roles = HashSet::new();
        let mut paths = HashSet::new();
        for file in &self.files {
            require(
                expected.contains(&file.role) && roles.insert(file.role),
                "wrong or repeated asset role",
            )?;
            require(paths.insert(&file.path), "package asset paths must differ")?;
            native_path(&file.path, host)?;
            require(file.bytes > 0, "asset size must be positive")?;
            sha256(&file.sha256)?;
        }
        if let Some(validation) = &self.validation {
            text(&validation.validator_version, 128)?;
            uuid(&validation.target_id)?;
            sha256(&validation.file_set_sha256)?;
            utc_timestamp(&validation.validated_at)?;
        }
        Ok(())
    }

    /// Exact hash-input bytes specified by the contract; no hash verification.
    pub fn file_set_bytes(&self) -> Result<Vec<u8>, LibraryError> {
        let mut files: Vec<_> = self.files.iter().collect();
        files.sort_by_key(|file| match file.role {
            FileRole::Config => "config",
            FileRole::Model => "model",
            FileRole::Voice => "voice",
        });
        Ok(serde_json::to_vec(&files)?)
    }
}

fn utc_timestamp(value: &str) -> Result<(), LibraryError> {
    text(value, 64)?;
    // UTC RFC 3339 with optional fractional seconds and Z or +00:00 offset.
    let value = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
        .ok_or(LibraryError::Invalid(
            "validation timestamp must be UTC RFC 3339",
        ))?;
    let (whole, fraction) = value
        .split_once('.')
        .map_or((value, None), |(w, f)| (w, Some(f)));
    require(
        whole.len() == 19
            && whole.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                10 => b == b'T',
                13 | 16 => b == b':',
                _ => b.is_ascii_digit(),
            })
            && fraction.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit())),
        "invalid validation timestamp",
    )?;
    let number = |range: std::ops::Range<usize>| whole[range].parse::<u32>().unwrap();
    let year = number(0..4);
    let month = number(5..7);
    let day = number(8..10);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    require(
        day >= 1
            && day <= days
            && number(11..13) < 24
            && number(14..16) < 60
            && number(17..19) <= 60,
        "validation timestamp has an invalid date or time",
    )
}
