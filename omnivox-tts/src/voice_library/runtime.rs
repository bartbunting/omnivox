use super::*;

/// Identity acknowledged by one server process, derived from exact input bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceLibraryConfiguration {
    pub target_id: String,
    pub profile_id: String,
    pub generation_id: String,
    pub sha256: String,
}

/// Configuration and administrative eligibility from one inventory snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceLibraryStatus {
    #[serde(deserialize_with = "required_nullable")]
    pub configuration: Option<VoiceLibraryConfiguration>,
    pub overridden_engines: Vec<String>,
    pub eligible_voices: Vec<PhysicalVoiceId>,
    pub inventory_generation: u64,
}

/// Serializable input. Use `RuntimeLibrary::parse` before consuming it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDocument {
    pub schema_version: u32,
    pub target_id: String,
    pub profile_id: String,
    pub generation_id: String,
    #[serde(deserialize_with = "deserialize_exclusions")]
    pub disabled_physical_ids: Vec<PhysicalVoiceId>,
    #[serde(deserialize_with = "required_nullable")]
    pub piper: Option<PiperLibrary>,
    #[serde(deserialize_with = "required_nullable")]
    pub flite: Option<FliteLibrary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiperLibrary {
    pub models: Vec<PiperModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiperModel {
    pub identity: ModelIdentity,
    pub model: AssetFile,
    pub config: AssetFile,
    pub voices: Vec<PiperVoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiperVoice {
    pub physical_id: String,
    pub speaker_index: u32,
    pub display_name: String,
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FliteLibrary {
    pub builtin_slt: bool,
    pub files: Vec<FliteVoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FliteVoice {
    pub physical_id: String,
    pub file: AssetFile,
    pub display_name: String,
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
}

/// Structurally validated, immutable runtime input and its exact original bytes.
/// Native validation and asset verification remain separate prerequisites.
#[derive(Debug, Clone)]
pub struct RuntimeLibrary {
    document: RuntimeDocument,
    source_bytes: Vec<u8>,
}

impl RuntimeLibrary {
    pub fn parse(bytes: &[u8], host: HostPlatform) -> Result<Self, LibraryError> {
        let document: RuntimeDocument = decode(bytes, MAX_RUNTIME_BYTES)?;
        document.validate(host)?;
        Ok(Self {
            document,
            source_bytes: bytes.to_vec(),
        })
    }

    /// Read at most the byte limit plus one overflow-detection byte.
    pub fn read(reader: impl Read, host: HostPlatform) -> Result<Self, LibraryError> {
        Self::parse(&read_bounded(reader, MAX_RUNTIME_BYTES)?, host)
    }

    /// Read a generation pinned by its trusted parent, before native loading.
    pub fn read_expected(
        reader: impl Read,
        host: HostPlatform,
        expected_sha256: Option<&str>,
    ) -> Result<Self, LibraryError> {
        let library = Self::read(reader, host)?;
        if let Some(expected) = expected_sha256 {
            sha256(expected)?;
            require(
                library.sha256() == expected,
                "generation SHA-256 differs from parent configuration",
            )?;
        }
        Ok(library)
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub fn configuration(&self) -> VoiceLibraryConfiguration {
        VoiceLibraryConfiguration {
            target_id: self.document.target_id.clone(),
            profile_id: self.document.profile_id.clone(),
            generation_id: self.document.generation_id.clone(),
            sha256: self.sha256(),
        }
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    /// Permission under this projection, not final runtime eligibility.
    /// The caller must resolve explicit provider overrides and engine policy
    /// separately. A null provider leaves legacy discovery in charge of its
    /// load set; none of these outcomes establishes native availability.
    pub fn permits(&self, voice: &PhysicalVoiceId) -> bool {
        if self.document.disabled_physical_ids.contains(voice) {
            return false;
        }
        match voice.engine_id.as_str() {
            "piper" => self.document.piper.as_ref().is_none_or(|piper| {
                piper
                    .models
                    .iter()
                    .any(|model| model.voices.iter().any(|v| v.physical_id == voice.voice_id))
            }),
            "flite" => self.document.flite.as_ref().is_none_or(|flite| {
                (flite.builtin_slt && voice.voice_id == "cmu_us_slt")
                    || flite.files.iter().any(|v| v.physical_id == voice.voice_id)
            }),
            _ => true,
        }
    }
}

impl RuntimeDocument {
    fn validate(&self, host: HostPlatform) -> Result<(), LibraryError> {
        require(self.schema_version == 1, "unsupported library schema")?;
        uuid(&self.target_id)?;
        uuid(&self.profile_id)?;
        uuid(&self.generation_id)?;
        exclusions(&self.disabled_physical_ids)?;
        let disabled = disabled_set(&self.disabled_physical_ids);
        let mut count = 0;
        if let Some(piper) = &self.piper {
            require(
                piper.models.len() <= MAX_PIPER_MODELS,
                "too many Piper models",
            )?;
            let mut previous_model = None;
            let mut voice_ids = HashSet::new();
            for model in &piper.models {
                model.identity.validate()?;
                let key = model.identity.sort_key();
                require(
                    previous_model.is_none_or(|old| old < key),
                    "Piper models must be sorted and unique",
                )?;
                previous_model = Some(key);
                model.model.validate(host)?;
                model.config.validate(host)?;
                require(
                    model.model.path != model.config.path,
                    "Piper model and configuration paths must differ",
                )?;
                require(
                    !model.voices.is_empty(),
                    "Piper model has no enabled speakers",
                )?;
                let mut previous_speaker = None;
                for voice in &model.voices {
                    model
                        .identity
                        .validate_binding(&voice.physical_id, voice.speaker_index)?;
                    voice_metadata(&voice.display_name, &voice.language)?;
                    require(
                        previous_speaker.is_none_or(|old| old < voice.speaker_index),
                        "Piper speakers must be sorted and unique",
                    )?;
                    previous_speaker = Some(voice.speaker_index);
                    require(
                        voice_ids.insert(&voice.physical_id),
                        "duplicate Piper voice ID",
                    )?;
                    require(
                        !disabled.contains(&("piper", voice.physical_id.as_str())),
                        "projected Piper voice is disabled",
                    )?;
                }
                count += model.voices.len();
            }
        }
        if let Some(flite) = &self.flite {
            require(flite.files.len() <= MAX_FLITE_FILES, "too many Flite files")?;
            if flite.builtin_slt {
                require(
                    !disabled.contains(&("flite", "cmu_us_slt")),
                    "projected built-in SLT is disabled",
                )?;
                count += 1;
            }
            let mut previous = None;
            for voice in &flite.files {
                flite_id(&voice.physical_id)?;
                require(
                    previous.is_none_or(|old| old < voice.physical_id.as_str()),
                    "Flite voices must be sorted and unique",
                )?;
                previous = Some(voice.physical_id.as_str());
                voice.file.validate(host)?;
                voice_metadata(&voice.display_name, &voice.language)?;
                require(
                    !disabled.contains(&("flite", voice.physical_id.as_str())),
                    "projected Flite voice is disabled",
                )?;
            }
            count += flite.files.len();
        }
        require(count <= MAX_PROJECTED_VOICES, "too many projected voices")
    }
}

pub(super) fn flite_id(value: &str) -> Result<(), LibraryError> {
    physical_id("flite", value)?;
    require(
        value
            .strip_prefix("flitevox:")
            .is_some_and(|v| !v.is_empty()),
        "invalid external Flite voice ID",
    )
}

/// A pointer record is data, not permission to commit or activate a generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePointer {
    pub schema_version: u32,
    pub target_id: String,
    pub profile_id: String,
    pub generation_id: String,
    pub sha256: String,
}

impl ActivePointer {
    pub fn parse(bytes: &[u8]) -> Result<Self, LibraryError> {
        let value: Self = decode(bytes, 4096)?;
        require(
            value.schema_version == 1,
            "unsupported active-pointer schema",
        )?;
        uuid(&value.target_id)?;
        uuid(&value.profile_id)?;
        uuid(&value.generation_id)?;
        sha256(&value.sha256)?;
        Ok(value)
    }
}
