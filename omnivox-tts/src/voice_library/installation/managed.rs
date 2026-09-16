use super::*;
use crate::voice_library::catalogue::Catalogue;

impl Profile {
    /// Publish one verified catalogue package, disabled, without changing active
    /// speech. Validation must have used its immutable final native paths.
    pub fn install_catalogue(
        &mut self,
        operation_id: &str,
        catalogue: &Catalogue,
        entry_id: &str,
        package_id: &str,
        revision_id: &str,
        expected: &str,
    ) -> Result<(), LibraryError> {
        uuid(operation_id)?;
        uuid(package_id)?;
        uuid(revision_id)?;
        self.check_index(expected)?;
        let entry = catalogue.entry(entry_id)?;
        require(
            !self.index.document().packages.iter().any(|package| {
                package.package_id == package_id || package.identity == entry.identity()
            }),
            "catalogue voice is already installed",
        )?;
        let operation = self.admission.completed_validation(operation_id)?;
        require(
            operation.plan().document().platform == std::env::consts::OS,
            "validation belongs to another native platform",
        )?;
        let evidence = operation.verified_validation()?;
        let library = operation.plan().generation();
        let root = self
            .path
            .parent()
            .and_then(Path::parent)
            .ok_or(LibraryError::Invalid("invalid managed profile root"))?;
        let directory = root.join("packages").join(package_id).join(revision_id);
        ordinary(&root.join("packages"), false)?;
        ordinary(&root.join("packages").join(package_id), false)?;
        ordinary(&directory, false)?;
        let expected_library = entry.generation(
            &self.index.document().target_id,
            &self.index.document().profile_id,
            &library.document().generation_id,
            &directory,
        )?;
        require(
            expected_library.document() == library.document(),
            "native validation does not match the catalogue package",
        )?;
        library.verify_assets(ProviderOverrides::default())?;
        // Retain and verify licence/model-card bytes as well as synthesis assets.
        for file in &entry.files {
            file.asset(&directory)?.open_verified()?;
        }
        require(
            read_bounded(
                open_file(&directory.join("catalogue.json"), false)?,
                catalogue::MAX_CATALOGUE_BYTES,
            )? == catalogue.source_bytes(),
            "managed catalogue provenance changed",
        )?;
        let mut files = Vec::new();
        for file in &entry.files {
            let role = match file.role.as_str() {
                "model" => FileRole::Model,
                "config" => FileRole::Config,
                "voice" => FileRole::Voice,
                _ => continue,
            };
            files.push(imports::file(role, &file.asset(&directory)?));
        }
        let mut package = PackageRevision {
            package_id: package_id.into(),
            revision_id: revision_id.into(),
            provider: entry.provider,
            ownership: Ownership::Managed,
            identity: entry.identity(),
            files,
            validation: None,
            catalogue: Some(CatalogueReference {
                revision: catalogue.document().revision.clone(),
                entry_id: entry_id.into(),
            }),
        };
        package.validation =
            Some(evidence.installation_validation(&package, self.admission.target_id())?);
        let mut document = self.index.document().clone();
        document.packages.push(package);
        for voice in &entry.voices {
            let id = PhysicalVoiceId::new(entry.engine_id(), &voice.physical_id);
            if !document.disabled_physical_ids.contains(&id) {
                document.disabled_physical_ids.push(id);
            }
            document.voices.push(imports::row(
                entry.engine_id(),
                &voice.physical_id,
                &voice.name,
                &Some(entry.language.clone()),
                voice.speaker_index,
                package_id,
                revision_id,
            ));
        }
        document.revision_id = local::new_uuid()?;
        sort_disabled(&mut document.disabled_physical_ids);
        let candidate = LibraryIndex::parse(&serde_json::to_vec(&document)?, host())?;
        #[derive(Serialize)]
        struct Receipt<'a> {
            schema_version: u32,
            operation_id: &'a str,
            validation_plan_sha256: String,
            catalogue_sha256: String,
            entry_id: &'a str,
            package_id: &'a str,
            revision_id: &'a str,
            previous_index_sha256: &'a str,
            index_sha256: String,
        }
        let receipt = Receipt {
            schema_version: 1,
            operation_id,
            validation_plan_sha256: operation.plan().sha256(),
            catalogue_sha256: digest(catalogue.source_bytes()),
            entry_id,
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
                .join(format!("{}.json", document.revision_id)),
            &serde_json::to_vec(&receipt)?,
        )?;
        self.replace_index(document, expected)
    }
}
