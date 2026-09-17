//! Explicit package removal with immutable review and resumable, bounded cleanup.
use super::*;
use crate::voice_library::{catalogue::Catalogue, retention};

mod references;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    operation_id: String,
    target_id: String,
    profile_id: String,
    previous_revision: String,
    previous_sha256: String,
    next_revision: String,
    package_id: String,
    revision_id: String,
    catalogue_json: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemovalReview {
    pub operation_id: String,
    pub plan_sha256: String,
    pub package_id: String,
    pub revision_id: String,
    pub name: String,
    pub voices: Vec<IndexedVoice>,
    pub package_bytes: u64,
    pub blockers: Vec<String>,
    pub detached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemovalResult {
    pub operation_id: String,
    pub status: String,
    /// Logical bytes unlinked with durable receipts, not filesystem free space.
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
    pub unconfirmed_bytes: u64,
    pub detail: String,
}

struct Removal {
    plan: Plan,
    hash: String,
    path: PathBuf,
    directory: PathBuf,
    package: PackageRevision,
    previous: LibraryIndex,
    next: LibraryIndex,
    files: Vec<AssetFile>,
    name: String,
}

impl Profile {
    fn storage_root(&self) -> Result<PathBuf, LibraryError> {
        let root = self
            .path
            .parent()
            .and_then(Path::parent)
            .ok_or(LibraryError::Invalid("invalid profile root"))?;
        // Avoid verbatim Windows syntax in retained metadata, including when a
        // resumed cleanup has already removed some of its files.
        Ok(PathBuf::from(catalogue::metadata_path(root)?))
    }

    pub fn prepare_removal(
        &self,
        voice: &PhysicalVoiceId,
        expected: &str,
    ) -> Result<RemovalReview, LibraryError> {
        self.check_index(expected)?;
        let root = self.storage_root()?;
        let _gate = retention::Gate::acquire(&root)?;
        let row = self
            .index
            .document()
            .voices
            .iter()
            .find(|row| row.engine_id == voice.engine_id && row.physical_id == voice.voice_id)
            .ok_or(LibraryError::Invalid(
                "voice is not installed in this profile",
            ))?;
        let package = self
            .index
            .document()
            .packages
            .iter()
            .find(|package| {
                Some(&package.package_id) == row.package_id.as_ref()
                    && Some(&package.revision_id) == row.revision_id.as_ref()
            })
            .ok_or(LibraryError::Invalid(
                "built-in and system voices cannot be uninstalled",
            ))?;
        require(
            package.ownership == Ownership::Managed,
            "imported files remain user-owned and cannot be uninstalled",
        )?;
        let directory = package_directory(&root, &package.package_id, &package.revision_id)?;
        check_directory(&root, &directory)?;
        let catalogue_json = String::from_utf8(read_bounded(
            open_file(&directory.join("catalogue.json"), false)?,
            catalogue::MAX_CATALOGUE_BYTES,
        )?)
        .map_err(|_| LibraryError::Invalid("managed catalogue is not UTF-8"))?;
        let plan = Plan {
            schema_version: 1,
            operation_id: local::new_uuid()?,
            target_id: self.index.document().target_id.clone(),
            profile_id: self.index.document().profile_id.clone(),
            previous_revision: self.index.document().revision_id.clone(),
            previous_sha256: expected.into(),
            next_revision: local::new_uuid()?,
            package_id: package.package_id.clone(),
            revision_id: package.revision_id.clone(),
            catalogue_json,
        };
        let parent = self.path.join("removals");
        if !parent.try_exists()? {
            super::directory(&parent)?;
        }
        ordinary(&parent, false)?;
        let path = parent.join(&plan.operation_id);
        super::directory(&path)?;
        save_new(&path.join("plan.json"), &serde_json::to_vec(&plan)?)?;
        let removal = self.read_removal(&plan.operation_id)?;
        self.review_removal(&removal)
    }

    pub fn pending_removals(&self) -> Result<Vec<RemovalReview>, LibraryError> {
        let root = self.storage_root()?;
        let _gate = retention::Gate::acquire(&root)?;
        let parent = self.path.join("removals");
        if !parent.try_exists()? {
            return Ok(Vec::new());
        }
        let mut reviews = Vec::new();
        for path in retention::entries(&parent, 4096)? {
            ordinary(&path, false)?;
            let operation = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(LibraryError::Invalid("invalid removal operation name"))?;
            let removal = self.read_removal(operation)?;
            // Unconfirmed deletion receipts remain available for inspection.
            if path.join("completed.json").try_exists()? {
                let completed: RemovalResult = decode(
                    &read_bounded(open_file(&path.join("completed.json"), false)?, 8192)?,
                    8192,
                )?;
                let observed = removal.result("complete", String::new())?;
                require(
                    completed.operation_id == operation
                        && completed.status == "complete"
                        && completed.remaining_bytes == 0
                        && observed.remaining_bytes == 0
                        && completed.removed_bytes == observed.removed_bytes
                        && completed.unconfirmed_bytes == observed.unconfirmed_bytes,
                    "completed removal differs from its deletion receipts",
                )?;
                continue;
            }
            if self.detached(&removal)?
                || self
                    .path
                    .join("index-revisions")
                    .join(format!("{}.json", removal.plan.next_revision))
                    .try_exists()?
            {
                require(reviews.len() < 128, "too many incomplete removals")?;
                reviews.push(self.review_removal(&removal)?);
            }
        }
        Ok(reviews)
    }

    fn read_removal(&self, operation: &str) -> Result<Removal, LibraryError> {
        uuid(operation)?;
        let root = self.storage_root()?;
        let parent = self.path.join("removals");
        ordinary(&parent, false)?;
        let path = parent.join(operation);
        ordinary(&path, false)?;
        let bytes = read_bounded(
            open_file(&path.join("plan.json"), false)?,
            2 * MAX_RUNTIME_BYTES,
        )?;
        let plan: Plan = decode(&bytes, 2 * MAX_RUNTIME_BYTES)?;
        require(
            plan.schema_version == 1
                && plan.operation_id == operation
                && plan.target_id == self.index.document().target_id
                && plan.profile_id == self.index.document().profile_id,
            "removal plan belongs to another operation or profile",
        )?;
        uuid(&plan.previous_revision)?;
        uuid(&plan.next_revision)?;
        sha256(&plan.previous_sha256)?;
        let previous = LibraryIndex::read(
            open_file(
                &self
                    .path
                    .join("index-revisions")
                    .join(format!("{}.json", plan.previous_revision)),
                false,
            )?,
            host(),
        )?;
        require(
            digest(previous.source_bytes()) == plan.previous_sha256
                && previous.document().target_id == plan.target_id
                && previous.document().profile_id == plan.profile_id,
            "removal ownership index changed",
        )?;
        let package = previous
            .document()
            .packages
            .iter()
            .find(|package| {
                package.package_id == plan.package_id && package.revision_id == plan.revision_id
            })
            .ok_or(LibraryError::Invalid(
                "removal package has no ownership record",
            ))?
            .clone();
        require(
            package.ownership == Ownership::Managed,
            "removal cannot own imported files",
        )?;
        let directory = package_directory(&root, &plan.package_id, &plan.revision_id)?;
        let catalogue = Catalogue::parse(plan.catalogue_json.as_bytes())?;
        let provenance = package.catalogue.as_ref().ok_or(LibraryError::Invalid(
            "managed catalogue provenance is missing",
        ))?;
        require(
            provenance.revision == catalogue.document().revision,
            "managed catalogue revision changed",
        )?;
        let entry = catalogue.entry(&provenance.entry_id)?;
        require(
            entry.provider == package.provider && entry.identity() == package.identity,
            "managed package identity differs from catalogue",
        )?;
        let mut files = Vec::new();
        for file in &entry.files {
            files.push(file.asset(&directory)?);
        }
        // Only exact installer-owned native files may be detached. Licence,
        // model-card and README files are separately pinned by the catalogue.
        let native: Vec<_> = entry
            .files
            .iter()
            .filter(|file| {
                matches!(
                    file.role.as_str(),
                    "model" | "config" | "voice" | "database"
                ) || file.role.starts_with("rhvoice/")
            })
            .map(|file| {
                let role = match file.role.as_str() {
                    "model" => FileRole::Model,
                    "config" => FileRole::Config,
                    "voice" => FileRole::Voice,
                    "database" => FileRole::Database,
                    _ => FileRole::RhvoiceData,
                };
                Ok(imports::file(role, &file.asset(&directory)?))
            })
            .collect::<Result<_, LibraryError>>()?;
        require(
            native == package.files,
            "managed package paths or hashes differ from catalogue",
        )?;
        files.push(AssetFile {
            path: directory.join("catalogue.json").to_string_lossy().into(),
            bytes: plan.catalogue_json.len() as u64,
            sha256: digest(plan.catalogue_json.as_bytes()),
        });
        let mut document = previous.document().clone();
        document.revision_id = plan.next_revision.clone();
        document.packages.retain(|item| {
            item.package_id != plan.package_id || item.revision_id != plan.revision_id
        });
        document.voices.retain(|row| {
            row.package_id.as_ref() != Some(&plan.package_id)
                || row.revision_id.as_ref() != Some(&plan.revision_id)
        });
        // Retain disabled physical IDs and palette choices, including after a
        // later re-download with new package/revision UUIDs.
        let next = LibraryIndex::parse(&serde_json::to_vec(&document)?, host())?;
        Ok(Removal {
            name: entry.name.clone(),
            plan,
            hash: digest(&bytes),
            path,
            directory,
            package,
            previous,
            next,
            files,
        })
    }

    fn detached(&self, removal: &Removal) -> Result<bool, LibraryError> {
        let archive = self
            .path
            .join("index-revisions")
            .join(format!("{}.json", removal.plan.next_revision));
        if !archive.try_exists()? {
            return Ok(false);
        }
        require(
            read_bounded(open_file(&archive, false)?, MAX_INDEX_BYTES)?
                == removal.next.source_bytes(),
            "removal index archive changed",
        )?;
        Ok(!self.index.document().packages.iter().any(|package| {
            package.package_id == removal.package.package_id
                && package.revision_id == removal.package.revision_id
        }) && self.index_sha256() != removal.plan.previous_sha256)
    }

    fn review_removal(&self, removal: &Removal) -> Result<RemovalReview, LibraryError> {
        let detached = self.detached(removal)?;
        let voices: Vec<_> = removal
            .previous
            .document()
            .voices
            .iter()
            .filter(|row| {
                row.package_id.as_ref() == Some(&removal.plan.package_id)
                    && row.revision_id.as_ref() == Some(&removal.plan.revision_id)
            })
            .cloned()
            .collect();
        let mut blockers = Vec::new();
        if !detached && self.index_sha256() != removal.plan.previous_sha256 {
            blockers.push("Desired voices changed; review uninstallation again".into());
        }
        if voices.iter().any(|row| row.enabled) {
            blockers.push(
                "Disable every listed speaker, then Apply before uninstalling this shared package"
                    .into(),
            );
        }
        // Hold all profile leases until callers finish the deletion. This
        // review alone is not authorization; execute repeats under those locks.
        match references::check(self, removal) {
            Ok((_profiles, reasons)) => blockers.extend(reasons),
            Err(error) => blockers.push(format!(
                "References could not be established; files retained: {error}"
            )),
        }
        if let Err(error) = removal.check_files(&self.storage_root()?, detached) {
            blockers.push(format!(
                "Package ownership could not be verified; files retained: {error}"
            ));
        }
        if let Err(error) = removal.result("review", String::new()) {
            blockers.push(format!(
                "Deletion evidence requires inspection; files retained: {error}"
            ));
        }
        Ok(RemovalReview {
            operation_id: removal.plan.operation_id.clone(),
            plan_sha256: removal.hash.clone(),
            package_id: removal.plan.package_id.clone(),
            revision_id: removal.plan.revision_id.clone(),
            name: removal.name.clone(),
            voices,
            package_bytes: removal.files.iter().map(|file| file.bytes).sum(),
            blockers,
            detached,
        })
    }

    pub fn execute_removal(
        &mut self,
        operation: &str,
        expected: &str,
    ) -> Result<RemovalResult, LibraryError> {
        let root = self.storage_root()?;
        let _gate = retention::Gate::acquire(&root)?;
        let removal = self.read_removal(operation)?;
        require(removal.hash == expected, "reviewed removal plan changed")?;
        let detached = self.detached(&removal)?;
        let review = self.review_removal(&removal)?;
        if !review.blockers.is_empty() {
            return removal.result("blocked", review.blockers.join("; "));
        }
        let (_profiles, blockers) = references::check(self, &removal)?;
        if !blockers.is_empty() {
            return removal.result("blocked", blockers.join("; "));
        }
        removal.check_files(&root, detached)?;
        if !detached {
            // Publish absence before unlinking. A crash cannot leave installed
            // metadata advertising a partly deleted model. Resume uses the
            // retained original ownership and exact next-index archive.
            let archive = self
                .path
                .join("index-revisions")
                .join(format!("{}.json", removal.plan.next_revision));
            if archive.try_exists()? {
                // A prior index write may have stopped before publication.
                require(
                    read_bounded(open_file(&archive, false)?, MAX_INDEX_BYTES)?
                        == removal.next.source_bytes(),
                    "removal archive changed",
                )?;
                let pending = self
                    .path
                    .join(format!("index-next-{}.json", removal.plan.next_revision));
                if pending.try_exists()? {
                    require(
                        read_bounded(open_file(&pending, false)?, MAX_INDEX_BYTES)?
                            == removal.next.source_bytes(),
                        "incomplete removal index publication; files retained",
                    )?;
                } else {
                    save_new(&pending, removal.next.source_bytes())?;
                }
                self.check_index(&removal.plan.previous_sha256)?;
                self.poisoned = true;
                fs::rename(&pending, self.path.join("index.json"))?;
                sync_directory(&self.path)?;
                self.index = LibraryIndex::parse(removal.next.source_bytes(), host())?;
                self.poisoned = false;
            } else {
                self.replace_index(
                    removal.next.document().clone(),
                    &removal.plan.previous_sha256,
                )?;
            }
        }
        let outcome = removal.delete_files(&root);
        let result = removal.result(
            if outcome.is_ok() {
                "complete"
            } else {
                "partial"
            },
            outcome.err().map_or_else(
                || "Managed package removed; saved voice choices retained".into(),
                |error| format!("Cleanup incomplete; remaining files retained: {error}"),
            ),
        )?;
        if result.status == "complete" && !removal.path.join("completed.json").try_exists()? {
            save_new(
                &removal.path.join("completed.json"),
                &serde_json::to_vec(&result)?,
            )?;
        }
        Ok(result)
    }
}

fn package_directory(root: &Path, package: &str, revision: &str) -> Result<PathBuf, LibraryError> {
    uuid(package)?;
    uuid(revision)?;
    Ok(root.join("packages").join(package).join(revision))
}
fn check_directory(root: &Path, directory: &Path) -> Result<(), LibraryError> {
    ordinary(&root.join("packages"), false)?;
    ordinary(directory.parent().unwrap(), false)?;
    ordinary(directory, false)
}

impl Removal {
    fn check_files(&self, root: &Path, detached: bool) -> Result<(), LibraryError> {
        if detached && !self.directory.try_exists()? {
            return Ok(());
        }
        check_directory(root, &self.directory)?;
        self.check_tree(&self.directory)?;
        for (index, file) in self.files.iter().enumerate() {
            let path = Path::new(&file.path);
            if detached && !path.try_exists()? {
                continue;
            }
            ordinary(path, true)?;
            require(
                !self.receipt(index).try_exists()?,
                "a deleted package file reappeared",
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                require(
                    fs::metadata(path)?.nlink() == 1,
                    "shared hard-linked package file retained",
                )?;
            }
            drop(file.open_verified()?);
        }
        Ok(())
    }
    fn directories(&self) -> std::collections::BTreeSet<PathBuf> {
        let mut directories = std::collections::BTreeSet::new();
        for file in &self.files {
            let mut parent = Path::new(&file.path).parent();
            while let Some(path) = parent {
                if path == self.directory {
                    break;
                }
                if !path.starts_with(&self.directory) {
                    break;
                }
                directories.insert(path.to_owned());
                parent = path.parent();
            }
        }
        directories
    }
    fn check_tree(&self, directory: &Path) -> Result<(), LibraryError> {
        let directories = self.directories();
        for path in retention::entries(directory, 256)? {
            if directories.contains(&path) {
                ordinary(&path, false)?;
                self.check_tree(&path)?;
            } else {
                ordinary(&path, true)?;
                require(
                    self.files.iter().any(|file| Path::new(&file.path) == path),
                    "unexpected package file; cleanup refused",
                )?;
            }
        }
        Ok(())
    }
    fn receipt(&self, index: usize) -> PathBuf {
        self.path.join(format!("deleted-{index:02}.json"))
    }
    fn delete_files(&self, root: &Path) -> Result<(), LibraryError> {
        for (index, file) in self.files.iter().enumerate() {
            if !Path::new(&file.path).try_exists()? {
                continue;
            }
            check_directory(root, &self.directory)?;
            for directory in self.directories() {
                if Path::new(&file.path).starts_with(&directory) {
                    ordinary(&directory, false)?;
                }
            }
            ordinary(Path::new(&file.path), true)?;
            drop(file.open_verified()?);
            fs::remove_file(&file.path)?;
            sync_directory(Path::new(&file.path).parent().unwrap())?;
            save_new(&self.receipt(index), &serde_json::to_vec(file)?)?;
        }
        for directory in self.directories().iter().rev() {
            if directory.try_exists()? {
                ordinary(directory, false)?;
                fs::remove_dir(directory)?;
            }
        }
        if self.directory.try_exists()? {
            fs::remove_dir(&self.directory)?;
        }
        // Empty package parents are tiny retained ownership locations. Never
        // recursively remove an unexpected file or another revision.
        sync_directory(self.directory.parent().unwrap())
    }
    fn result(&self, status: &str, detail: String) -> Result<RemovalResult, LibraryError> {
        let (mut removed, mut remaining, mut unconfirmed) = (0, 0, 0);
        for (index, file) in self.files.iter().enumerate() {
            if self.receipt(index).try_exists()? {
                let saved = read_bounded(open_file(&self.receipt(index), false)?, 8192)?;
                require(
                    saved == serde_json::to_vec(file)? && !Path::new(&file.path).try_exists()?,
                    "deletion receipt or asset changed",
                )?;
                removed += file.bytes;
            } else if Path::new(&file.path).try_exists()? {
                remaining += file.bytes;
            } else {
                unconfirmed += file.bytes;
            }
        }
        Ok(RemovalResult {
            operation_id: self.plan.operation_id.clone(),
            status: status.into(),
            removed_bytes: removed,
            remaining_bytes: remaining,
            unconfirmed_bytes: unconfirmed,
            detail,
        })
    }
}
