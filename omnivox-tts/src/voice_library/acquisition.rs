//! Durable acquisition metadata and private storage. No network or synthesis.
use super::catalogue::Catalogue;
use super::local::Host;
use super::operations::storage::{new_file, open_file, ordinary};
use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcquisitionPlan {
    pub schema_version: u32,
    pub operation_id: String,
    pub target_id: String,
    pub profile_id: String,
    pub package_id: String,
    pub revision_id: String,
    pub generation_id: String,
    pub expected_index_sha256: String,
    pub catalogue_json: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub operation_id: String,
    pub state: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub terminal: bool,
    pub detail: String,
}

pub struct Acquisition {
    pub plan: AcquisitionPlan,
    pub directory: PathBuf,
    pub staging: PathBuf,
    pub package: PathBuf,
    journal: File,
    lease: File,
    records: usize,
}
impl Drop for Acquisition {
    fn drop(&mut self) {
        let _ = self.lease.unlock();
    }
}

impl Acquisition {
    pub fn prepare(
        host: &Host,
        catalogue: &Catalogue,
        entry_id: &str,
    ) -> Result<Self, LibraryError> {
        let entry = catalogue.entry(entry_id)?;
        let lock_path = host
            .root
            .join("profiles")
            .join(&host.profile_id)
            .join("acquire.lock");
        let lease = match new_file(&lock_path) {
            Ok(file) => file,
            Err(LibraryError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                open_file(&lock_path, true)?
            }
            Err(error) => return Err(error),
        };
        lease
            .try_lock()
            .map_err(|_| LibraryError::Invalid("a voice installation is already running"))?;
        let profile = host.profile()?;
        require(
            !profile
                .index()
                .document()
                .packages
                .iter()
                .any(|package| package.identity == entry.identity()),
            "this catalogue voice is already installed",
        )?;
        let plan = AcquisitionPlan {
            schema_version: 1,
            operation_id: local::new_uuid()?,
            target_id: host.target_id.clone(),
            profile_id: host.profile_id.clone(),
            package_id: local::new_uuid()?,
            revision_id: local::new_uuid()?,
            generation_id: local::new_uuid()?,
            expected_index_sha256: profile.index_sha256(),
            catalogue_json: String::from_utf8(catalogue.source_bytes().to_vec())
                .map_err(|_| LibraryError::Invalid("catalogue is not UTF-8"))?,
            entry_id: entry_id.into(),
        };
        drop(profile);
        for name in ["acquisitions", "staging", "packages"] {
            ensure_directory(&host.root.join(name))?;
        }
        let directory = host.root.join("acquisitions").join(&plan.operation_id);
        create_directory(&directory)?;
        let staging = host.root.join("staging").join(&plan.operation_id);
        create_directory(&staging)?;
        let package_parent = host.root.join("packages").join(&plan.package_id);
        create_directory(&package_parent)?;
        let package = package_parent.join(&plan.revision_id);
        write_new(&directory.join("plan.json"), &serde_json::to_vec(&plan)?)?;
        let journal = new_file(&directory.join("events.jsonl"))?;
        Ok(Self {
            plan,
            directory,
            staging,
            package,
            journal,
            lease,
            records: 0,
        })
    }

    pub fn record(&mut self, progress: &Progress) -> Result<(), LibraryError> {
        require(
            self.records < 2048 && progress.operation_id == self.plan.operation_id,
            "acquisition progress exceeds bound or belongs to another operation",
        )?;
        text(&progress.state, 64)?;
        text(&progress.detail, 2048)?;
        require(
            progress.downloaded_bytes <= progress.total_bytes,
            "invalid download progress",
        )?;
        let mut bytes = serde_json::to_vec(progress)?;
        require(bytes.len() <= 4096, "acquisition event exceeds bound")?;
        bytes.push(b'\n');
        self.journal.write_all(&bytes)?;
        self.journal.sync_all()?;
        self.records += 1;
        Ok(())
    }

    /// Publish immutable file locations before native validation, still absent
    /// from the installed index. Interrupted/unindexed packages are retained.
    pub fn place_files(&self, catalogue: &Catalogue) -> Result<(), LibraryError> {
        for file in &catalogue.entry(&self.plan.entry_id)?.files {
            file.asset(&self.staging)?.open_verified()?;
        }
        write_new(
            &self.staging.join("catalogue.json"),
            catalogue.source_bytes(),
        )?;
        require(
            !self.package.try_exists()?,
            "managed package destination already exists",
        )?;
        fs::rename(&self.staging, &self.package)?;
        #[cfg(unix)]
        File::open(self.package.parent().unwrap())?.sync_all()?;
        Ok(())
    }
}

pub fn inspect(host: &Host, operation: &str) -> Result<Vec<Progress>, LibraryError> {
    uuid(operation)?;
    let directory = host.root.join("acquisitions").join(operation);
    ordinary(&directory, false)?;
    let plan: AcquisitionPlan = decode(
        &read_bounded(
            open_file(&directory.join("plan.json"), false)?,
            2 * MAX_RUNTIME_BYTES,
        )?,
        2 * MAX_RUNTIME_BYTES,
    )?;
    require(
        plan.operation_id == operation
            && plan.target_id == host.target_id
            && plan.profile_id == host.profile_id,
        "acquisition belongs to another profile",
    )?;
    let bytes = read_bounded(
        open_file(&directory.join("events.jsonl"), false)?,
        8 * MAX_RUNTIME_BYTES,
    )?;
    require(
        bytes.last().is_none_or(|byte| *byte == b'\n'),
        "interrupted acquisition journal write; retain for inspection",
    )?;
    let mut records = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event: Progress = decode(line, 4096)?;
        require(
            event.operation_id == operation && records.len() < 2048,
            "invalid acquisition journal",
        )?;
        records.push(event);
    }
    Ok(records)
}

pub fn new_download(path: &Path) -> Result<File, LibraryError> {
    new_file(path)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let mut file = new_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn ensure_directory(path: &Path) -> Result<(), LibraryError> {
    match create_directory(path) {
        Err(LibraryError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ordinary(path, false)
        }
        result => result,
    }
}
fn create_directory(path: &Path) -> Result<(), LibraryError> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staging_is_exclusive_verified_and_never_installed_implicitly() {
        let path = std::env::temp_dir().join(format!(
            "omnivox-acquire-test-{}",
            local::new_uuid().unwrap()
        ));
        let host = Host::open(&path).unwrap();
        let catalogue =
            Catalogue::parse(catalogue::tests::fixture().to_string().as_bytes()).unwrap();
        let mut operation = Acquisition::prepare(&host, &catalogue, "flite-test").unwrap();
        assert!(Acquisition::prepare(&host, &catalogue, "flite-test").is_err());
        let event = Progress {
            operation_id: operation.plan.operation_id.clone(),
            state: "downloading".into(),
            downloaded_bytes: 0,
            total_bytes: 3,
            terminal: false,
            detail: "Downloading".into(),
        };
        operation.record(&event).unwrap();
        assert_eq!(inspect(&host, &event.operation_id).unwrap().len(), 1);
        assert!(operation.place_files(&catalogue).is_err());
        fs::write(operation.staging.join("voice.flitevox"), b"bad").unwrap();
        assert!(operation.place_files(&catalogue).is_err());
        fs::write(operation.staging.join("voice.flitevox"), b"abc").unwrap();
        operation.place_files(&catalogue).unwrap();
        assert!(operation.package.join("catalogue.json").is_file());
        assert!(!operation.staging.exists());
        // Windows canonical roots carry a verbatim prefix. Generation metadata
        // must retain the same files using the contract's ordinary path syntax.
        catalogue
            .entry("flite-test")
            .unwrap()
            .generation(
                &host.target_id,
                &host.profile_id,
                &operation.plan.generation_id,
                &operation.package,
            )
            .unwrap()
            .verify_assets(ProviderOverrides::default())
            .unwrap();

        let mut profile = host.profile().unwrap();
        assert!(profile.index().document().voices.is_empty());
        // Exact hashes alone cannot publish a voice without native validation.
        assert!(profile
            .install_catalogue(
                &event.operation_id,
                &catalogue,
                "flite-test",
                &operation.plan.package_id,
                &operation.plan.revision_id,
                &operation.plan.expected_index_sha256
            )
            .is_err());
        assert!(profile.index().document().voices.is_empty());
        drop(profile);
        drop(operation);
        drop(Acquisition::prepare(&host, &catalogue, "flite-test").unwrap());
        fs::remove_dir_all(path).unwrap();
    }
}
