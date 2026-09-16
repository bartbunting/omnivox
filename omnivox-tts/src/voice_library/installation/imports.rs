use super::*;

#[derive(Serialize)]
struct ImportReceipt<'a> {
    schema_version: u32,
    operation_id: &'a str,
    plan_sha256: String,
    package_id: &'a str,
    revision_id: &'a str,
    previous_index_sha256: &'a str,
    index_sha256: String,
}

impl Profile {
    /// Register a new local Piper model or external Flite voice after admitted
    /// native validation. Every imported voice starts disabled. No asset is
    /// copied, moved, deleted, or silently adopted from legacy configuration.
    pub fn import_validated(
        &mut self,
        operation_id: &str,
        package_id: &str,
        revision_id: &str,
        index_revision_id: &str,
        expected: &str,
    ) -> Result<(), LibraryError> {
        for id in [operation_id, package_id, revision_id, index_revision_id] {
            uuid(id)?;
        }
        self.check_index(expected)?;
        require(
            !self
                .index
                .document()
                .packages
                .iter()
                .any(|package| package.package_id == package_id),
            "package already exists; import requires a new package identity",
        )?;
        let operation: Operation = self.admission.completed_validation(operation_id)?;
        require(
            operation.plan().document().platform == std::env::consts::OS,
            "validation belongs to another native platform",
        )?;
        let evidence = operation.verified_validation()?;
        let library = operation.plan().generation();
        require(
            library.validation_targets().len() == 1,
            "import validation must select exactly one external model or voice",
        )?;
        library.verify_assets(ProviderOverrides::default())?;
        let generation = library.document();
        let mut voices = Vec::new();
        let (provider, identity, files) = if let Some(model) = generation
            .piper
            .as_ref()
            .and_then(|piper| piper.models.first())
        {
            require(
                matches!(model.identity, ModelIdentity::Import { .. }),
                "local imports require an import identity, not catalogue provenance",
            )?;
            for voice in &model.voices {
                require(
                    voice.physical_id == model.identity.piper_voice_id(voice.speaker_index)?,
                    "legacy voice adoption requires a separate impact plan",
                )?;
                voices.push(row(
                    "piper",
                    &voice.physical_id,
                    &voice.display_name,
                    &voice.language,
                    Some(voice.speaker_index),
                    package_id,
                    revision_id,
                ));
            }
            (
                Provider::Piper,
                model.identity.clone(),
                vec![
                    file(FileRole::Model, &model.model),
                    file(FileRole::Config, &model.config),
                ],
            )
        } else {
            let voice = generation
                .flite
                .as_ref()
                .and_then(|flite| flite.files.first())
                .ok_or(LibraryError::Invalid("built-in voices cannot be imported"))?;
            voices.push(row(
                "flite",
                &voice.physical_id,
                &voice.display_name,
                &voice.language,
                None,
                package_id,
                revision_id,
            ));
            (
                Provider::Flite,
                ModelIdentity::Import {
                    import_id: package_id.into(),
                },
                vec![file(FileRole::Voice, &voice.file)],
            )
        };
        let mut package = PackageRevision {
            package_id: package_id.into(),
            revision_id: revision_id.into(),
            provider,
            ownership: Ownership::Imported,
            identity,
            files,
            validation: None,
            catalogue: None,
        };
        package.validation =
            Some(evidence.installation_validation(&package, self.admission.target_id())?);
        let mut document = self.index.document().clone();
        document.packages.push(package);
        for voice in voices {
            let id = PhysicalVoiceId::new(&voice.engine_id, &voice.physical_id);
            if !document.disabled_physical_ids.contains(&id) {
                document.disabled_physical_ids.push(id);
            }
            document.voices.push(voice);
        }
        document.revision_id = index_revision_id.into();
        sort_disabled(&mut document.disabled_physical_ids);
        let candidate = LibraryIndex::parse(&serde_json::to_vec(&document)?, host())?;
        require(
            index_revision_id != self.index.document().revision_id,
            "import requires a new index revision",
        )?;
        let receipt = ImportReceipt {
            schema_version: 1,
            operation_id,
            plan_sha256: operation.plan().sha256(),
            package_id,
            revision_id,
            previous_index_sha256: expected,
            index_sha256: digest(candidate.source_bytes()),
        };
        self.check_index(expected)?;
        save_new(
            &self
                .path
                .join("imports")
                .join(format!("{index_revision_id}.json")),
            &serde_json::to_vec(&receipt)?,
        )?;
        self.replace_index(document, expected)
    }
}

fn file(role: FileRole, asset: &AssetFile) -> IndexedFile {
    IndexedFile {
        role,
        path: asset.path.clone(),
        bytes: asset.bytes,
        sha256: asset.sha256.clone(),
    }
}

fn row(
    engine: &str,
    id: &str,
    name: &str,
    language: &Option<String>,
    speaker: Option<u32>,
    package: &str,
    revision: &str,
) -> IndexedVoice {
    IndexedVoice {
        physical_id: id.into(),
        engine_id: engine.into(),
        display_name: name.into(),
        language: language.clone(),
        enabled: false,
        package_id: Some(package.into()),
        revision_id: Some(revision.into()),
        speaker_index: speaker,
        legacy_physical_id: None,
    }
}
