//! Reviewed acquisition data. Parsing never downloads or loads a voice.
use super::*;
use std::path::Path;

pub const MAX_CATALOGUE_BYTES: usize = 1024 * 1024;
pub const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueDocument {
    pub schema_version: u32,
    pub revision: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub provider: Provider,
    pub name: String,
    pub language: String,
    pub description: String,
    pub source: String,
    pub source_revision: String,
    pub licence: String,
    pub licence_url: String,
    pub files: Vec<DownloadFile>,
    pub voices: Vec<CatalogueVoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadFile {
    pub role: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueVoice {
    pub physical_id: String,
    pub name: String,
    #[serde(deserialize_with = "required_nullable")]
    pub speaker_index: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Catalogue {
    document: CatalogueDocument,
    source: Vec<u8>,
}

impl Catalogue {
    pub fn parse(bytes: &[u8]) -> Result<Self, LibraryError> {
        let document: CatalogueDocument = decode(bytes, MAX_CATALOGUE_BYTES)?;
        require(
            matches!(document.schema_version, 1 | 2),
            "unknown voice catalogue schema",
        )?;
        text(&document.revision, 128)?;
        require(
            document.entries.len() <= 128,
            "voice catalogue has too many entries",
        )?;
        let mut ids = HashSet::new();
        let mut physical = HashSet::new();
        for entry in &document.entries {
            require(
                entry.provider != Provider::Mbrola || document.schema_version == 2,
                "MBROLA requires catalogue schema 2",
            )?;
            entry.validate()?;
            require(ids.insert(&entry.id), "duplicate catalogue identity")?;
            for voice in &entry.voices {
                require(
                    physical.insert((entry.provider.engine_id(), &voice.physical_id)),
                    "duplicate catalogue physical voice",
                )?;
            }
        }
        Ok(Self {
            document,
            source: bytes.to_vec(),
        })
    }
    pub fn document(&self) -> &CatalogueDocument {
        &self.document
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }
    pub fn entry(&self, id: &str) -> Result<&Entry, LibraryError> {
        self.document
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(LibraryError::Invalid(
                "voice is absent from the reviewed catalogue",
            ))
    }
}

impl DownloadFile {
    /// Fixed role names, never a filename or path supplied by remote metadata.
    pub fn filename(&self) -> Result<&'static str, LibraryError> {
        match self.role.as_str() {
            "model" => Ok("model.onnx"),
            "config" => Ok("model.onnx.json"),
            "voice" => Ok("voice.flitevox"),
            "database" => Ok("database.mbrola"),
            "readme" => Ok("README"),
            "model_card" => Ok("MODEL_CARD"),
            "licence" => Ok("LICENSE"),
            _ => Err(LibraryError::Invalid("unsupported catalogue file role")),
        }
    }
    pub fn asset(&self, directory: &Path) -> Result<AssetFile, LibraryError> {
        Ok(AssetFile {
            path: metadata_path(&directory.join(self.filename()?))?,
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        })
    }
}

/// Serialize a native-selected path using the ordinary absolute path syntax
/// accepted by voice metadata. Windows canonicalization adds a verbatim prefix;
/// remove it only after proving that ordinary Win32 resolution is unchanged.
pub fn metadata_path(path: &Path) -> Result<String, LibraryError> {
    let value = path
        .to_str()
        .ok_or(LibraryError::Invalid("voice path is not UTF-8"))?
        .to_owned();
    #[cfg(windows)]
    let value = {
        let ordinary = if let Some(tail) = value.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{tail}")
        } else if let Some(tail) = value.strip_prefix(r"\\?\") {
            tail.to_owned()
        } else {
            value.to_owned()
        };
        if ordinary != value {
            require(
                Path::new(&ordinary).canonicalize()? == path.canonicalize()?,
                "voice path requires unsupported verbatim Windows semantics",
            )?;
        }
        ordinary
    };
    native_path(
        &value,
        if cfg!(windows) {
            HostPlatform::Windows
        } else {
            HostPlatform::Posix
        },
    )?;
    Ok(value)
}

/// HTTPS is required except for the upstream Flite repository, which currently
/// serves its published voices only over HTTP. Every payload is checksum-pinned.
pub fn source_url(url: &str) -> Result<(), LibraryError> {
    text(url, 4096)?;
    require(
        url.is_ascii() && !url.contains(['\\', '#', ' ']),
        "invalid catalogue URL",
    )?;
    let rest = url
        .strip_prefix("https://")
        .or_else(|| {
            url.strip_prefix("http://festvox.org/flite/packed/")
                .map(|_| "festvox.org/")
        })
        .ok_or(LibraryError::Invalid(
            "catalogue source requires HTTPS or the pinned FestVox voice repository",
        ))?;
    let authority = rest.split('/').next().unwrap_or_default();
    require(
        !authority.is_empty()
            && !authority.contains(['@', ':', '?'])
            && authority
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte)),
        "invalid catalogue source host",
    )
}

impl Entry {
    pub fn engine_id(&self) -> &'static str {
        self.provider.engine_id()
    }
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
    pub fn identity(&self) -> ModelIdentity {
        ModelIdentity::Catalogue {
            catalogue_key: self.id.clone(),
        }
    }

    fn validate(&self) -> Result<(), LibraryError> {
        self.identity().validate()?;
        for (value, limit) in [
            (&self.name, 256),
            (&self.language, 64),
            (&self.description, 2048),
            (&self.source_revision, 128),
            (&self.licence, 1024),
        ] {
            text(value, limit)?;
        }
        source_url(&self.source)?;
        source_url(&self.licence_url)?;
        require(
            (1..=5).contains(&self.files.len()),
            "invalid catalogue file count",
        )?;
        let mut roles = HashSet::new();
        for file in &self.files {
            file.filename()?;
            require(
                roles.insert(file.role.as_str()),
                "duplicate catalogue file role",
            )?;
            source_url(&file.url)?;
            sha256(&file.sha256)?;
            require(
                file.bytes > 0 && file.bytes <= MAX_DOWNLOAD_BYTES,
                "invalid catalogue file size",
            )?;
            if matches!(
                file.role.as_str(),
                "config" | "model_card" | "licence" | "readme"
            ) {
                require(
                    file.bytes <= MAX_RUNTIME_BYTES as u64,
                    "catalogue metadata exceeds bound",
                )?;
            }
        }
        require(
            self.total_bytes() <= MAX_DOWNLOAD_BYTES,
            "catalogue package exceeds download bound",
        )?;
        require(
            (1..=MAX_PROJECTED_VOICES).contains(&self.voices.len()),
            "invalid catalogue voice count",
        )?;
        match self.provider {
            Provider::Piper => require(
                roles.contains("model")
                    && roles.contains("config")
                    && roles.contains("model_card")
                    && !roles.contains("voice")
                    && !roles.contains("database"),
                "incomplete Piper catalogue package",
            )?,
            Provider::Mbrola => require(
                roles.len() == 3
                    && roles.contains("database")
                    && roles.contains("licence")
                    && roles.contains("readme")
                    && self.voices.len() == 1,
                "invalid MBROLA catalogue package",
            )?,
            Provider::Flite => require(
                roles.contains("voice")
                    && !roles.contains("model")
                    && !roles.contains("config")
                    && !roles.contains("database")
                    && self.voices.len() == 1,
                "invalid Flite catalogue package",
            )?,
        }
        for voice in &self.voices {
            text(&voice.name, 256)?;
            match self.provider {
                Provider::Piper => require(
                    voice.speaker_index.is_some_and(|index| {
                        self.identity()
                            .piper_voice_id(index)
                            .is_ok_and(|id| id == voice.physical_id)
                    }),
                    "catalogue Piper speaker identity mismatch",
                )?,
                Provider::Mbrola => {
                    require(
                        voice.speaker_index.is_none() && voice.physical_id != MBROLA_EN1,
                        "MBROLA downloads must be external voices without a speaker index",
                    )?;
                    let profile = MbrolaProfile::for_id(&voice.physical_id)?;
                    let database = self
                        .files
                        .iter()
                        .find(|file| file.role == "database")
                        .unwrap();
                    require(
                        database.sha256 == profile.database_sha256,
                        "catalogue MBROLA database/profile mismatch",
                    )?;
                }
                Provider::Flite => {
                    text(&voice.physical_id, 256)?;
                    require(
                        voice.speaker_index.is_none()
                            && voice.physical_id.starts_with("flitevox:")
                            && voice.physical_id.len() > 9,
                        "invalid catalogue Flite identity",
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn generation(
        &self,
        target: &str,
        profile: &str,
        generation: &str,
        directory: &Path,
    ) -> Result<RuntimeLibrary, LibraryError> {
        self.validate()?;
        let asset = |role: &str| {
            self.files
                .iter()
                .find(|file| file.role == role)
                .ok_or(LibraryError::Invalid("missing catalogue asset"))?
                .asset(directory)
        };
        let mut document = RuntimeDocument {
            schema_version: if self.provider == Provider::Mbrola {
                2
            } else {
                1
            },
            target_id: target.into(),
            profile_id: profile.into(),
            generation_id: generation.into(),
            disabled_physical_ids: vec![],
            piper: None,
            flite: None,
            mbrola: None,
        };
        match self.provider {
            Provider::Piper => {
                document.piper = Some(PiperLibrary {
                    models: vec![PiperModel {
                        identity: self.identity(),
                        model: asset("model")?,
                        config: asset("config")?,
                        voices: self
                            .voices
                            .iter()
                            .map(|voice| PiperVoice {
                                physical_id: voice.physical_id.clone(),
                                speaker_index: voice.speaker_index.expect("validated speaker"),
                                display_name: voice.name.clone(),
                                language: Some(self.language.clone()),
                            })
                            .collect(),
                    }],
                })
            }
            Provider::Mbrola => {
                document.mbrola = Some(MbrolaLibrary {
                    builtin_en1: false,
                    files: vec![MbrolaVoice {
                        physical_id: self.voices[0].physical_id.clone(),
                        database: asset("database")?,
                        display_name: self.voices[0].name.clone(),
                        language: Some(self.language.clone()),
                    }],
                });
            }
            Provider::Flite => {
                document.flite = Some(FliteLibrary {
                    builtin_slt: false,
                    files: vec![FliteVoice {
                        physical_id: self.voices[0].physical_id.clone(),
                        file: asset("voice")?,
                        display_name: self.voices[0].name.clone(),
                        language: Some(self.language.clone()),
                    }],
                })
            }
        }
        RuntimeLibrary::parse(
            &serde_json::to_vec(&document)?,
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub fn fixture() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"revision":"test", "entries":[{
            "id":"flite-test","provider":"flite","name":"Test","language":"en",
            "description":"Test voice","source":"https://example.org/voices",
            "source_revision":"one","licence":"Test terms","licence_url":"https://example.org/licence",
            "files":[{"role":"voice","url":"https://example.org/voice","bytes":3,
                "sha256":verification::digest(b"abc")}],
            "voices":[{"physical_id":"flitevox:test","name":"Test","speaker_index":null}]
        }]})
    }
    #[test]
    fn rejects_ambiguous_or_unbounded_catalogue_inputs() {
        let value = fixture();
        Catalogue::parse(value.to_string().as_bytes()).unwrap();
        let mut cases = Vec::new();
        let mut v = value.clone();
        v["entries"][0]["files"][0]["role"] = "../voice".into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["files"][0]["bytes"] = (MAX_DOWNLOAD_BYTES + 1).into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["files"][0]["url"] = "http://example.org/voice".into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["files"][0]["url"] = "https://user:password@example.org/voice".into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["provider"] = "piper".into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["voices"][0]["speaker_index"] = 1.into();
        cases.push(v);
        let mut v = value.clone();
        v["entries"]
            .as_array_mut()
            .unwrap()
            .push(value["entries"][0].clone());
        cases.push(v);
        let mut v = value.clone();
        v["entries"][0]["unexpected"] = true.into();
        cases.push(v);
        for v in cases {
            assert!(
                Catalogue::parse(v.to_string().as_bytes()).is_err(),
                "accepted {v}"
            );
        }
        let duplicate = value.to_string().replace(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
        );
        assert!(Catalogue::parse(duplicate.as_bytes()).is_err());
        assert!(source_url("http://festvox.org/flite/packed/voice").is_ok());
        assert!(source_url("http://festvox.org.evil/flite/packed/voice").is_err());
    }
}
