//! Native, profile-locked installation and activation preparation.
//! Imported assets stay in place. These operations never restart speech.
use super::operations::storage::{new_file, open_file, ordinary};
use super::operations::{Admission, Operation};
use super::verification::digest;
use super::*;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

mod imports;
#[cfg(test)]
mod tests;

fn host() -> HostPlatform {
    if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Posix
    }
}

/// Retains the same native profile lease used by validation. The provider must
/// select and initialize this target/root; caller UUIDs do not authenticate it.
pub struct Profile {
    admission: Admission,
    path: PathBuf,
    index: LibraryIndex,
    poisoned: bool,
}

/// Frozen preparation for the client's explicit two-lane Apply. A candidate is
/// not an applied generation and is never sufficient evidence to commit one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationCandidate {
    pub schema_version: u32,
    pub configuration: VoiceLibraryConfiguration,
    pub index_revision_id: String,
    pub index_sha256: String,
    #[serde(deserialize_with = "required_nullable")]
    pub previous_active_json: Option<String>,
}
impl ActivationCandidate {
    pub fn to_bytes(&self) -> Result<Vec<u8>, LibraryError> {
        Ok(serde_json::to_vec(self)?)
    }
}

impl Profile {
    /// Initialize desired storage under an existing validation-admission profile.
    /// Existing or partial storage is retained and never silently reinitialized.
    pub fn initialize(root: &Path, profile: &str, revision: &str) -> Result<Self, LibraryError> {
        uuid(profile)?;
        uuid(revision)?;
        let admission =
            Admission::try_open(root, profile)?.ok_or(LibraryError::Invalid("profile is busy"))?;
        let path = profile_path(root, profile)?;
        require(
            !path.join("index.json").try_exists()?,
            "voice library is already initialized",
        )?;
        for name in ["index-revisions", "imports", "generations", "candidates"] {
            directory(&path.join(name))?;
        }
        let document = IndexDocument {
            schema_version: 1,
            target_id: admission.target_id().into(),
            profile_id: profile.into(),
            revision_id: revision.into(),
            packages: Vec::new(),
            voices: Vec::new(),
            disabled_physical_ids: Vec::new(),
        };
        let index = LibraryIndex::parse(&serde_json::to_vec(&document)?, host())?;
        save_new(
            &path
                .join("index-revisions")
                .join(format!("{revision}.json")),
            index.source_bytes(),
        )?;
        save_new(&path.join("index.json"), index.source_bytes())?;
        Ok(Self {
            admission,
            path,
            index,
            poisoned: false,
        })
    }

    pub fn open(root: &Path, profile: &str) -> Result<Self, LibraryError> {
        uuid(profile)?;
        let admission =
            Admission::try_open(root, profile)?.ok_or(LibraryError::Invalid("profile is busy"))?;
        let path = profile_path(root, profile)?;
        for name in ["index-revisions", "imports", "generations", "candidates"] {
            ordinary(&path.join(name), false)?;
        }
        let index = LibraryIndex::read(open_file(&path.join("index.json"), false)?, host())?;
        require(
            index.document().target_id == admission.target_id()
                && index.document().profile_id == profile,
            "installed index belongs to another target/profile",
        )?;
        let archived = read_bounded(
            open_file(
                &path
                    .join("index-revisions")
                    .join(format!("{}.json", index.document().revision_id)),
                false,
            )?,
            MAX_INDEX_BYTES,
        )?;
        require(
            archived == index.source_bytes(),
            "installed index differs from its retained revision",
        )?;
        Ok(Self {
            admission,
            path,
            index,
            poisoned: false,
        })
    }

    pub fn index(&self) -> &LibraryIndex {
        &self.index
    }
    pub fn index_sha256(&self) -> String {
        digest(self.index.source_bytes())
    }

    fn check_index(&self, expected: &str) -> Result<(), LibraryError> {
        require(
            !self.poisoned,
            "profile write outcome requires reopening and inspection",
        )?;
        sha256(expected)?;
        require(
            self.index_sha256() == expected,
            "desired voice state changed; recompute the plan",
        )?;
        let current = read_bounded(
            open_file(&self.path.join("index.json"), false)?,
            MAX_INDEX_BYTES,
        )?;
        require(
            current == self.index.source_bytes(),
            "installed index changed outside its owner",
        )
    }

    fn replace_index(
        &mut self,
        document: IndexDocument,
        expected: &str,
    ) -> Result<(), LibraryError> {
        self.check_index(expected)?;
        require(
            document.revision_id != self.index.document().revision_id,
            "index changes require a new revision UUID",
        )?;
        let index = LibraryIndex::parse(&serde_json::to_vec(&document)?, host())?;
        let revision = &index.document().revision_id;
        // An immutable snapshot preserves both sides of each desired-state edit.
        // The same-directory rename publishes one complete index to readers.
        save_new(
            &self
                .path
                .join("index-revisions")
                .join(format!("{revision}.json")),
            index.source_bytes(),
        )?;
        let pending = self.path.join(format!("index-next-{revision}.json"));
        save_new(&pending, index.source_bytes())?;
        self.check_index(expected)?;
        self.poisoned = true;
        fs::rename(&pending, self.path.join("index.json"))?;
        sync_directory(&self.path)?;
        self.index = index;
        self.poisoned = false;
        Ok(())
    }

    /// Save desired enablement only. The current active pointer and live lanes
    /// retain their previous configuration until the client explicitly applies.
    pub fn set_enabled(
        &mut self,
        voice: &PhysicalVoiceId,
        enabled: bool,
        revision: &str,
        expected: &str,
    ) -> Result<(), LibraryError> {
        uuid(revision)?;
        self.check_index(expected)?;
        let mut document = self.index.document().clone();
        let row = document
            .voices
            .iter_mut()
            .find(|row| row.engine_id == voice.engine_id && row.physical_id == voice.voice_id)
            .ok_or(LibraryError::Invalid(
                "voice is not installed in this profile",
            ))?;
        row.enabled = enabled;
        document.disabled_physical_ids.retain(|id| id != voice);
        if !enabled {
            document.disabled_physical_ids.push(voice.clone());
        }
        sort_disabled(&mut document.disabled_physical_ids);
        document.revision_id = revision.into();
        self.replace_index(document, expected)
    }

    /// Write an immutable enabled-only generation and a frozen activation plan.
    /// Native startup/status and the two-lane restart still belong to Apply.
    pub fn stage_activation(
        &self,
        generation: &str,
        piper: bool,
        flite: bool,
        expected: &str,
    ) -> Result<ActivationCandidate, LibraryError> {
        self.check_index(expected)?;
        let library = self.index.project(generation, piper, flite, host())?;
        library.verify_assets(ProviderOverrides::default())?;
        let candidate = ActivationCandidate {
            schema_version: 1,
            configuration: library.configuration(),
            index_revision_id: self.index.document().revision_id.clone(),
            index_sha256: expected.into(),
            previous_active_json: self.active_json()?,
        };
        self.check_index(expected)?;
        save_new(&self.generation_path(generation), library.source_bytes())?;
        save_new(
            &self
                .path
                .join("candidates")
                .join(format!("{generation}.json")),
            &candidate.to_bytes()?,
        )?;
        Ok(candidate)
    }

    /// Recheck a saved candidate against desired state, its exact generation and
    /// the previous active pointer before handing it to the activation client.
    pub fn activation_candidate(
        &self,
        generation: &str,
    ) -> Result<ActivationCandidate, LibraryError> {
        uuid(generation)?;
        let bytes = read_bounded(
            open_file(
                &self
                    .path
                    .join("candidates")
                    .join(format!("{generation}.json")),
                false,
            )?,
            16 * 1024,
        )?;
        let candidate: ActivationCandidate = decode(&bytes, 16 * 1024)?;
        require(
            candidate.schema_version == 1
                && candidate.index_revision_id == self.index.document().revision_id,
            "activation candidate index changed",
        )?;
        self.check_index(&candidate.index_sha256)?;
        let library =
            RuntimeLibrary::read(open_file(&self.generation_path(generation), false)?, host())?;
        require(
            library.configuration() == candidate.configuration
                && candidate.configuration.generation_id == generation
                && candidate.configuration.target_id == self.admission.target_id()
                && candidate.configuration.profile_id == self.index.document().profile_id,
            "activation candidate generation changed",
        )?;
        require(
            candidate.previous_active_json == self.active_json()?,
            "active voice generation changed; recompute the plan",
        )?;
        library.verify_assets(ProviderOverrides::default())?;
        Ok(candidate)
    }

    fn generation_path(&self, generation: &str) -> PathBuf {
        self.path
            .join("generations")
            .join(format!("{generation}.json"))
    }

    fn active_json(&self) -> Result<Option<String>, LibraryError> {
        let path = self.path.join("active.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(_) => (),
        }
        let bytes = read_bounded(open_file(&path, false)?, 4096)?;
        let active = ActivePointer::parse(&bytes)?;
        require(
            active.target_id == self.admission.target_id()
                && active.profile_id == self.index.document().profile_id,
            "active pointer belongs to another target/profile",
        )?;
        let library = RuntimeLibrary::read(
            open_file(&self.generation_path(&active.generation_id), false)?,
            host(),
        )?;
        require(
            library.sha256() == active.sha256
                && library.document().generation_id == active.generation_id
                && library.document().target_id == active.target_id
                && library.document().profile_id == active.profile_id,
            "active pointer generation changed",
        )?;
        Ok(Some(String::from_utf8(bytes).map_err(|_| {
            LibraryError::Invalid("active pointer is not UTF-8")
        })?))
    }
}

fn profile_path(root: &Path, profile: &str) -> Result<PathBuf, LibraryError> {
    let path = root.join("profiles").join(profile);
    ordinary(&path, false)?;
    Ok(path.canonicalize()?)
}
fn sort_disabled(ids: &mut [PhysicalVoiceId]) {
    ids.sort_by(|a, b| (&a.engine_id, &a.voice_id).cmp(&(&b.engine_id, &b.voice_id)));
}
fn directory(path: &Path) -> Result<(), LibraryError> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)?;
    sync_directory(path.parent().unwrap())
}
fn save_new(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let mut file = new_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    sync_directory(path.parent().unwrap())
}
fn sync_directory(path: &Path) -> Result<(), LibraryError> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
