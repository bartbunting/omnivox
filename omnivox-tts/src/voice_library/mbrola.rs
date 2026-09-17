//! Reviewed bindings between frontend profiles and database bytes.
use super::*;

pub const MBROLA_EN1: &str = "mbrola:v1/mb-en1/en1";

#[derive(Debug, Clone, Copy)]
pub struct MbrolaProfile {
    pub physical_id: &'static str,
    pub frontend: &'static str,
    pub database_sha256: &'static str,
    pub sample_rate: u32,
}

impl MbrolaProfile {
    pub fn for_id(id: &str) -> Result<Self, LibraryError> {
        let (physical_id, frontend, database_sha256) = match id {
            MBROLA_EN1 => (
                MBROLA_EN1,
                "mb-en1",
                "edb8eaae6f0e38493d88ed627518632e6ff8a3843bcf08474a1a70aa786fd99f",
            ),
            "mbrola:v1/mb-us1/us1" => (
                "mbrola:v1/mb-us1/us1",
                "mb-us1",
                "9f1cd90de6334f43cb4f7348cacd0806fb414b0fbc762492bde7000b9e192a9a",
            ),
            "mbrola:v1/mb-us2/us2" => (
                "mbrola:v1/mb-us2/us2",
                "mb-us2",
                "75f7f6b605945f4b65713c3ad1fba5be38fb720cde01bdc3d2facf988634c0d5",
            ),
            "mbrola:v1/mb-us3/us3" => (
                "mbrola:v1/mb-us3/us3",
                "mb-us3",
                "7cc5c49e098f80091e34ed0bd50f0af3908ac2427fbbe7d2b16c57defb7611ec",
            ),
            _ => {
                return Err(LibraryError::Invalid(
                    "unreviewed MBROLA frontend/database binding",
                ))
            }
        };
        Ok(Self {
            physical_id,
            frontend,
            database_sha256,
            sample_rate: 16000,
        })
    }

    pub fn validate_database(&self, file: &AssetFile) -> Result<(), LibraryError> {
        require(
            file.sha256 == self.database_sha256,
            "MBROLA database does not match its frontend profile",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MbrolaLibrary {
    pub builtin_en1: bool,
    pub files: Vec<MbrolaVoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MbrolaVoice {
    pub physical_id: String,
    pub database: AssetFile,
    pub display_name: String,
    #[serde(deserialize_with = "required_nullable")]
    pub language: Option<String>,
}

impl MbrolaLibrary {
    pub(super) fn validate(
        &self,
        host: HostPlatform,
        disabled: &HashSet<(&str, &str)>,
    ) -> Result<usize, LibraryError> {
        require(self.files.len() <= 4, "too many MBROLA databases")?;
        require(
            !self.builtin_en1 || !disabled.contains(&("mbrola", MBROLA_EN1)),
            "projected built-in en1 is disabled",
        )?;
        let mut previous = None;
        for voice in &self.files {
            let profile = MbrolaProfile::for_id(&voice.physical_id)?;
            require(
                previous.is_none_or(|old| old < voice.physical_id.as_str()),
                "MBROLA voices must be sorted and unique",
            )?;
            previous = Some(voice.physical_id.as_str());
            require(
                !self.builtin_en1 || voice.physical_id != MBROLA_EN1,
                "duplicate built-in en1",
            )?;
            voice.database.validate(host)?;
            profile.validate_database(&voice.database)?;
            voice_metadata(&voice.display_name, &voice.language)?;
            require(
                !disabled.contains(&("mbrola", voice.physical_id.as_str())),
                "projected MBROLA voice is disabled",
            )?;
        }
        Ok(self.files.len() + usize::from(self.builtin_en1))
    }
    pub fn voice_ids(&self) -> impl Iterator<Item = &str> {
        self.builtin_en1
            .then_some(MBROLA_EN1)
            .into_iter()
            .chain(self.files.iter().map(|voice| voice.physical_id.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const TARGET: &str = "11111111-1111-4111-8111-111111111111";
    const PROFILE: &str = "22222222-2222-4222-8222-222222222222";
    const GENERATION: &str = "33333333-3333-4333-8333-333333333333";
    const US1: &str = "mbrola:v1/mb-us1/us1";
    fn generation() -> serde_json::Value {
        json!({"schema_version":2,"target_id":TARGET,"profile_id":PROFILE,"generation_id":GENERATION,
            "disabled_physical_ids":[],"piper":null,"flite":null,
            "mbrola":{"builtin_en1":true,"files":[{"physical_id":US1,"display_name":"US1","language":"en-US",
                "database":{"path":"/voices/us1","bytes":7238094,"sha256":MbrolaProfile::for_id(US1).unwrap().database_sha256}}]}})
    }
    fn parse(value: &serde_json::Value) -> Result<RuntimeLibrary, LibraryError> {
        RuntimeLibrary::parse(&serde_json::to_vec(value).unwrap(), HostPlatform::Posix)
    }
    #[test]
    fn en1_and_us1_are_independently_eligible_and_validated() {
        let library = parse(&generation()).unwrap();
        let targets = library.validation_targets();
        assert_eq!(
            targets,
            vec![
                PhysicalVoiceId::new("mbrola", MBROLA_EN1),
                PhysicalVoiceId::new("mbrola", US1)
            ]
        );
        for target in targets {
            assert!(library.permits(&target));
            let unit = library.validation_unit(&target).unwrap();
            assert_eq!(unit.validation_targets(), vec![target.clone()]);
            let policy = VoiceEligibility::from_library(&unit, ProviderOverrides::default());
            assert!(policy.permits(&target));
            let other = if target.voice_id == US1 {
                MBROLA_EN1
            } else {
                US1
            };
            assert!(!policy.permits(&PhysicalVoiceId::new("mbrola", other)));
        }
    }
    #[test]
    fn database_mismatch_alias_duplicate_and_disabled_loads_are_rejected() {
        let source = generation();
        let mut old_schema = source.clone();
        old_schema["schema_version"] = 1.into();
        assert!(parse(&old_schema).is_err());
        let mut wrong = source.clone();
        wrong["mbrola"]["files"][0]["database"]["sha256"] = MbrolaProfile::for_id(MBROLA_EN1)
            .unwrap()
            .database_sha256
            .into();
        assert!(parse(&wrong).is_err());
        let mut alias = source.clone();
        alias["mbrola"]["files"][0]["physical_id"] = "mb-us1".into();
        assert!(parse(&alias).is_err());
        let mut duplicate = source.clone();
        duplicate["mbrola"]["files"]
            .as_array_mut()
            .unwrap()
            .push(source["mbrola"]["files"][0].clone());
        assert!(parse(&duplicate).is_err());
        for id in [MBROLA_EN1, US1] {
            let mut disabled = source.clone();
            disabled["disabled_physical_ids"] = json!([{"engine_id":"mbrola","voice_id":id}]);
            assert!(parse(&disabled).is_err());
        }
        let mut empty = source;
        empty["mbrola"] = json!({"builtin_en1":false,"files":[]});
        let library = parse(&empty).unwrap();
        let policy = VoiceEligibility::from_library(&library, ProviderOverrides::default());
        assert!(policy.excludes_provider("mbrola"));
        assert!(!policy.permits(&PhysicalVoiceId::new("mbrola", US1)));
    }
    #[test]
    fn old_generations_keep_legacy_discovery_and_serialization() {
        let mut old = generation();
        old.as_object_mut().unwrap().remove("mbrola");
        old["schema_version"] = 1.into();
        let library = parse(&old).unwrap();
        assert!(library.permits(&PhysicalVoiceId::new("mbrola", MBROLA_EN1)));
        assert_eq!(serde_json::to_value(library.document()).unwrap(), old);
    }
    #[test]
    fn only_enabled_databases_are_projected_and_validation_is_required() {
        let mut package = PackageRevision {
            package_id: "44444444-4444-4444-8444-444444444444".into(),
            revision_id: GENERATION.into(),
            provider: Provider::Mbrola,
            ownership: Ownership::Managed,
            identity: ModelIdentity::Catalogue {
                catalogue_key: "mbrola-us1".into(),
            },
            files: vec![IndexedFile {
                role: FileRole::Database,
                path: "/voices/us1".into(),
                bytes: 7238094,
                sha256: MbrolaProfile::for_id(US1).unwrap().database_sha256.into(),
            }],
            catalogue: Some(CatalogueReference {
                revision: "test".into(),
                entry_id: "mbrola-us1".into(),
            }),
            validation: None,
        };
        package.validation = Some(NativeValidation {
            validator_version: "test".into(),
            target_id: TARGET.into(),
            validated_at: "2026-09-17T00:00:00Z".into(),
            file_set_sha256: package.file_set_sha256().unwrap(),
        });
        let builtin = IndexedVoice {
            physical_id: MBROLA_EN1.into(),
            engine_id: "mbrola".into(),
            display_name: "en1".into(),
            language: Some("en-GB".into()),
            enabled: true,
            package_id: None,
            revision_id: None,
            speaker_index: None,
            legacy_physical_id: None,
        };
        let mut us1 = builtin.clone();
        us1.physical_id = US1.into();
        us1.package_id = Some(package.package_id.clone());
        us1.revision_id = Some(package.revision_id.clone());
        us1.enabled = false;
        let mut document = IndexDocument {
            schema_version: 2,
            target_id: TARGET.into(),
            profile_id: PROFILE.into(),
            revision_id: GENERATION.into(),
            packages: vec![package],
            voices: vec![builtin, us1],
            disabled_physical_ids: vec![PhysicalVoiceId::new("mbrola", US1)],
        };
        let project = |document: &IndexDocument| {
            LibraryIndex::parse(&serde_json::to_vec(document).unwrap(), HostPlatform::Posix)
                .unwrap()
                .project(GENERATION, false, false, true, false, HostPlatform::Posix)
        };
        assert_eq!(
            project(&document).unwrap().validation_targets(),
            vec![PhysicalVoiceId::new("mbrola", MBROLA_EN1)]
        );
        document.voices[1].enabled = true;
        document.disabled_physical_ids.clear();
        assert_eq!(project(&document).unwrap().validation_targets().len(), 2);
        document.packages[0].validation = None;
        assert!(project(&document).is_err());
    }
}
