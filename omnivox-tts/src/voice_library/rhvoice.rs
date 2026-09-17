//! Managed RHVoice resources supplement, but never own, the external runtime.
use super::*;

pub const MAX_RHVOICE_FILES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RhvoiceLibrary {
    /// Ordinary startup preserves separately installed voices. Native validation
    /// uses only its selected package and never falls back to external data.
    pub inherit_external: bool,
    pub voices: Vec<RhvoiceVoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RhvoiceVoice {
    pub physical_id: String,
    pub display_name: String,
    pub language: Option<String>,
    pub files: Vec<AssetFile>,
}

pub fn rhvoice_id(value: &str) -> Result<(), LibraryError> {
    physical_id("rhvoice", value)?;
    require(
        value.strip_prefix("rhvoice:").is_some_and(|name| {
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        }),
        "invalid RHVoice physical ID",
    )
}

/// The initial reviewed resource layout is deliberately smaller than a general
/// archive extractor. Remote data cannot choose arbitrary installation paths.
pub fn resource_path(value: &str) -> Result<(), LibraryError> {
    let name = value
        .strip_prefix("rhvoice/")
        .ok_or(LibraryError::Invalid("invalid RHVoice resource path"))?;
    let valid = if let Some(name) = name.strip_prefix("language/") {
        matches!(
            name,
            "accents.dt"
                | "cmulex.fst"
                | "cmulex.lts"
                | "downcase.fst"
                | "emoji.fst"
                | "gpos.fst"
                | "key.fst"
                | "labelling.xml"
                | "language.info"
                | "lseq.fst"
                | "msg.fst"
                | "numbers.fst"
                | "phonemes.xml"
                | "phrasing.dt"
                | "spell.fst"
                | "syl.fst"
                | "tok.fst"
                | "tones.dt"
                | "vocab.fst"
        )
    } else if let Some(name) = name.strip_prefix("voice/") {
        matches!(name, "voice.info" | "voice.params")
            || name.split_once('/').is_some_and(|(rate, file)| {
                matches!(rate, "16000" | "24000")
                    && matches!(
                        file,
                        "voice.data"
                            | "bpf.txt"
                            | "bap.pdf"
                            | "dur.pdf"
                            | "lf0.pdf"
                            | "mgc.pdf"
                            | "bap.win1"
                            | "bap.win2"
                            | "bap.win3"
                            | "lf0.win1"
                            | "lf0.win2"
                            | "lf0.win3"
                            | "mgc.win1"
                            | "mgc.win2"
                            | "mgc.win3"
                            | "tree-bap.inf"
                            | "tree-dur.inf"
                            | "tree-lf0.inf"
                            | "tree-mgc.inf"
                    )
            })
    } else {
        false
    };
    require(valid, "unsupported RHVoice resource file")
}

impl RhvoiceVoice {
    pub(super) fn verify_files(&self) -> Result<(), LibraryError> {
        use std::path::{Path, PathBuf};
        let root = PathBuf::from(self.root()?).join("rhvoice");
        let expected: HashSet<_> = self.files.iter().map(|f| PathBuf::from(&f.path)).collect();
        let mut directories = HashSet::from([root.clone()]);
        for file in &expected {
            let mut parent = file.parent();
            while let Some(path) = parent {
                if path == root {
                    break;
                }
                require(
                    path.starts_with(&root),
                    "RHVoice file outside resource root",
                )?;
                directories.insert(path.to_owned());
                parent = path.parent();
            }
        }
        for directory in &directories {
            super::operations::storage::ordinary(directory, false)?;
            for entry in std::fs::read_dir(directory)? {
                let path = entry?.path();
                require(
                    expected.contains(&path) || directories.contains(&path),
                    "unlisted RHVoice resource",
                )?;
            }
        }
        for file in &self.files {
            super::operations::storage::ordinary(Path::new(&file.path), true)?;
            file.open_verified()?;
        }
        Ok(())
    }

    pub fn root(&self) -> Result<String, LibraryError> {
        self.files
            .iter()
            .find_map(|file| {
                let path = file.path.replace('\\', "/");
                path.strip_suffix("/rhvoice/voice/voice.info")
                    .map(str::to_owned)
            })
            .ok_or(LibraryError::Invalid("RHVoice package lacks voice.info"))
    }

    pub(super) fn validate(&self, host: HostPlatform) -> Result<(), LibraryError> {
        rhvoice_id(&self.physical_id)?;
        voice_metadata(&self.display_name, &self.language)?;
        require(
            (1..=MAX_RHVOICE_FILES).contains(&self.files.len()),
            "invalid RHVoice resource count",
        )?;
        let prefix = self.root()? + "/";
        let mut names = HashSet::new();
        for file in &self.files {
            file.validate(host)?;
            let path = file.path.replace('\\', "/");
            let relative = path.strip_prefix(&prefix).ok_or(LibraryError::Invalid(
                "RHVoice resources cross package roots",
            ))?;
            resource_path(relative)?;
            require(
                names.insert(relative.to_owned()),
                "duplicate RHVoice resource",
            )?;
        }
        require(
            names.contains("rhvoice/language/language.info")
                && names.contains("rhvoice/voice/voice.params"),
            "incomplete RHVoice resources",
        )
    }
}

impl RhvoiceLibrary {
    pub(super) fn validate(
        &self,
        host: HostPlatform,
        disabled: &HashSet<(&str, &str)>,
    ) -> Result<usize, LibraryError> {
        require(self.voices.len() <= 16, "too many RHVoice voices")?;
        let mut previous = None;
        for voice in &self.voices {
            voice.validate(host)?;
            require(
                previous.is_none_or(|old| old < voice.physical_id.as_str()),
                "RHVoice voices must be sorted and unique",
            )?;
            previous = Some(voice.physical_id.as_str());
            require(
                !disabled.contains(&("rhvoice", voice.physical_id.as_str())),
                "projected RHVoice voice is disabled",
            )?;
        }
        Ok(self.voices.len())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;

    pub fn catalogue_fixture() -> serde_json::Value {
        let mut value = catalogue::tests::fixture();
        value["schema_version"] = 3.into();
        let entry = &mut value["entries"][0];
        entry["id"] = "rhvoice-test".into();
        entry["provider"] = "rhvoice".into();
        entry["voices"] =
            json!([{"physical_id":"rhvoice:Alan","name":"Alan","speaker_index":null}]);
        entry["files"] = ["licence", "readme", "rhvoice/language/language.info", "rhvoice/voice/voice.info", "rhvoice/voice/voice.params"]
            .iter().map(|role| json!({"role":role,"url":"https://example.org/data","bytes":3,"sha256":verification::digest(b"abc")})).collect();
        value
    }

    #[test]
    fn catalogue_rejects_older_schema_and_unreviewed_resource_paths() {
        let source = catalogue_fixture();
        catalogue::Catalogue::parse(source.to_string().as_bytes()).unwrap();
        for path in [
            "rhvoice/../voice.info",
            "rhvoice/voice/../../escape",
            "rhvoice/voice/voice.conf",
            "rhvoice/voice/16000/../voice.data",
            "rhvoice/voice/C:/bad",
            "rhvoice/voice/voice.info:stream",
        ] {
            let mut value = source.clone();
            value["entries"][0]["files"][2]["role"] = path.into();
            assert!(
                catalogue::Catalogue::parse(value.to_string().as_bytes()).is_err(),
                "{path}"
            );
        }
        let mut old = source;
        old["schema_version"] = 2.into();
        assert!(catalogue::Catalogue::parse(old.to_string().as_bytes()).is_err());
    }

    #[test]
    fn native_validation_isolated_from_external_voices_and_unlisted_files() {
        let root =
            std::env::temp_dir().join(format!("rhvoice-test-{}", local::new_uuid().unwrap()));
        let catalogue =
            catalogue::Catalogue::parse(catalogue_fixture().to_string().as_bytes()).unwrap();
        let entry = catalogue.entry("rhvoice-test").unwrap();
        for file in &entry.files {
            let path = root.join(file.filename().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"abc").unwrap();
        }
        let id = local::new_uuid().unwrap();
        let library = entry.generation(&id, &id, &id, &root).unwrap();
        library.verify_assets(Default::default()).unwrap();
        assert!(!library.permits(&PhysicalVoiceId::new("rhvoice", "rhvoice:SLT")));
        let mut document = library.document().clone();
        document.rhvoice.as_mut().unwrap().inherit_external = true;
        let inherited = RuntimeLibrary::parse(
            &serde_json::to_vec(&document).unwrap(),
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )
        .unwrap();
        assert!(inherited.permits(&PhysicalVoiceId::new("rhvoice", "rhvoice:SLT")));
        let target = PhysicalVoiceId::new("rhvoice", "rhvoice:Alan");
        let unit = inherited.validation_unit(&target).unwrap();
        assert!(unit.permits(&target));
        assert!(!unit.permits(&PhysicalVoiceId::new("rhvoice", "rhvoice:SLT")));
        assert!(
            inherited
                .document()
                .rhvoice
                .as_ref()
                .unwrap()
                .inherit_external
        );
        std::fs::write(root.join("rhvoice/voice/voice.conf"), b"unreviewed").unwrap();
        assert!(library.verify_assets(Default::default()).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
