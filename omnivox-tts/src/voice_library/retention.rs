//! Serialize package cleanup with publication of native startup references.
//! Missing retirement evidence retains a snapshot, including older owners.
use super::operations::storage::{new_file, open_file, ordinary};
use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Gate(File);
impl Gate {
    fn inspect(root: &Path) -> Result<Self, LibraryError> {
        ordinary(root, false)?;
        let file = open_file(&root.join("retention.lock"), true)?;
        file.try_lock()
            .map_err(|_| LibraryError::Invalid("voice storage is busy; retry"))?;
        Ok(Self(file))
    }

    pub fn acquire(root: &Path) -> Result<Self, LibraryError> {
        ordinary(root, false)?;
        let path = root.join("retention.lock");
        let file = match new_file(&path) {
            Ok(file) => file,
            Err(LibraryError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                open_file(&path, true)?
            }
            Err(error) => return Err(error),
        };
        file.try_lock()
            .map_err(|_| LibraryError::Invalid("voice storage is busy; retry"))?;
        Ok(Self(file))
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    snapshot_sha256: String,
    state: String,
}

/// Called only after the live owner has confirmed retirement of its complete
/// worker tree and output readers. Dropping a process or releasing a lock is
/// insufficient evidence, so there is deliberately no automatic Drop receipt.
pub fn retired(host: &local::Host, snapshot: &Path, expected: &str) -> Result<(), LibraryError> {
    let _gate = Gate::acquire(&host.root)?;
    record(host, snapshot, expected, "retired")
}

/// Reconcile a lost owner reply using its existing, exact retirement receipt.
/// A prepared snapshot, missing evidence or process exit is not retirement.
/// This never changes a record, signals a process or releases an asset claim.
pub fn retirement_confirmed(
    host: &local::Host,
    worker: &str,
    snapshot: &Path,
    expected: &str,
) -> Result<bool, LibraryError> {
    uuid(worker)?;
    sha256(expected)?;
    require(
        snapshot == host.root.join("sessions").join(format!("{worker}.json")),
        "startup snapshot is outside this host or belongs to another worker",
    )?;
    let _gate = Gate::inspect(&host.root)?;
    let bytes = read_bounded(open_file(snapshot, false)?, local::MAX_STARTUP_BYTES)?;
    require(
        verification::digest(&bytes) == expected,
        "startup snapshot changed before retirement inspection",
    )?;
    let path = snapshot.with_extension("retired");
    if !path.try_exists()? {
        return Ok(false);
    }
    let receipt: Receipt = decode(&read_bounded(open_file(&path, false)?, 4096)?, 4096)?;
    require(
        receipt.schema_version == 1
            && receipt.state == "retired"
            && receipt.snapshot_sha256 == expected,
        "invalid startup retirement receipt",
    )?;
    Ok(true)
}

pub(super) fn prepared(
    host: &local::Host,
    snapshot: &Path,
    expected: &str,
) -> Result<(), LibraryError> {
    record(host, snapshot, expected, "prepared")
}

fn record(
    host: &local::Host,
    snapshot: &Path,
    expected: &str,
    state: &str,
) -> Result<(), LibraryError> {
    sha256(expected)?;
    let worker = snapshot
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(LibraryError::Invalid("invalid startup snapshot name"))?;
    uuid(worker)?;
    require(
        snapshot == host.root.join("sessions").join(format!("{worker}.json")),
        "startup snapshot is outside this host",
    )?;
    let bytes = read_bounded(open_file(snapshot, false)?, local::MAX_STARTUP_BYTES)?;
    require(
        verification::digest(&bytes) == expected,
        "startup snapshot changed before retirement",
    )?;
    let receipt = Receipt {
        schema_version: 1,
        snapshot_sha256: expected.into(),
        state: state.into(),
    };
    let bytes = serde_json::to_vec(&receipt)?;
    let path = snapshot.with_extension(state);
    if path.try_exists()? {
        require(
            read_bounded(open_file(&path, false)?, 4096)? == bytes,
            "startup receipt changed",
        )?;
    } else {
        let mut file = new_file(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        File::open(path.parent().unwrap())?.sync_all()?;
    }
    Ok(())
}

/// A prepared snapshot has never owned a worker; reusing it creates a new
/// independently pinned owner snapshot before opening the native START gate.
pub(super) fn released(snapshot: &Path, bytes: &[u8]) -> Result<bool, LibraryError> {
    for state in ["retired", "prepared"] {
        let path = snapshot.with_extension(state);
        if path.try_exists()? {
            let receipt: Receipt = decode(&read_bounded(open_file(&path, false)?, 4096)?, 4096)?;
            require(
                receipt.schema_version == 1
                    && receipt.state == state
                    && receipt.snapshot_sha256 == verification::digest(bytes),
                "invalid startup retirement/preparation receipt",
            )?;
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn entries(path: &Path, limit: usize) -> Result<Vec<PathBuf>, LibraryError> {
    ordinary(path, false)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        require(
            result.len() < limit,
            "voice storage inspection exceeds its bound",
        )?;
        result.push(entry?.path());
    }
    result.sort();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        host: local::Host,
        worker: String,
        snapshot: PathBuf,
        digest: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("omnivox-retirement-{}", local::new_uuid().unwrap()));
            let host = local::Host::open(&root).unwrap();
            drop(Gate::acquire(&host.root).unwrap());
            let worker = local::new_uuid().unwrap();
            let snapshot = host.root.join("sessions").join(format!("{worker}.json"));
            let bytes = b"synthetic snapshot bytes; no native process";
            new_file(&snapshot).unwrap().write_all(bytes).unwrap();
            Self {
                host,
                worker,
                snapshot,
                digest: verification::digest(bytes),
            }
        }
        fn inspect(&self) -> Result<bool, LibraryError> {
            retirement_confirmed(&self.host, &self.worker, &self.snapshot, &self.digest)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.host.root);
        }
    }

    #[test]
    fn only_exact_retirement_evidence_confirms_cleanup_without_mutation() {
        let f = Fixture::new();
        assert!(!f.inspect().unwrap());
        prepared(&f.host, &f.snapshot, &f.digest).unwrap();
        assert!(!f.inspect().unwrap());
        retired(&f.host, &f.snapshot, &f.digest).unwrap();
        let original = fs::read(f.snapshot.with_extension("retired")).unwrap();
        assert!(f.inspect().unwrap());
        assert!(f.inspect().unwrap());
        assert_eq!(
            fs::read(f.snapshot.with_extension("retired")).unwrap(),
            original
        );
        let other = local::new_uuid().unwrap();
        assert!(retirement_confirmed(&f.host, &other, &f.snapshot, &f.digest).is_err());
        assert!(retirement_confirmed(&f.host, &f.worker, &f.snapshot, &"0".repeat(64)).is_err());
        let other_host = Fixture::new();
        assert!(retirement_confirmed(&other_host.host, &f.worker, &f.snapshot, &f.digest).is_err());
    }

    #[test]
    fn changed_or_partial_evidence_never_becomes_cleanup() {
        let f = Fixture::new();
        retired(&f.host, &f.snapshot, &f.digest).unwrap();
        let path = f.snapshot.with_extension("retired");
        let original = fs::read(&path).unwrap();
        for bytes in [b"{".as_slice(), b"{}", b"[]"] {
            fs::write(&path, bytes).unwrap();
            assert!(f.inspect().is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let wrong = Receipt {
            schema_version: 1,
            state: "prepared".into(),
            snapshot_sha256: f.digest.clone(),
        };
        fs::write(&path, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(f.inspect().is_err());
        fs::write(&path, &original).unwrap();
        fs::write(&f.snapshot, b"changed").unwrap();
        assert!(f.inspect().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn busy_or_missing_retention_lock_does_not_get_replaced() {
        let f = Fixture::new();
        retired(&f.host, &f.snapshot, &f.digest).unwrap();
        let gate = Gate::acquire(&f.host.root).unwrap();
        assert!(f.inspect().is_err());
        drop(gate);
        let path = f.host.root.join("retention.lock");
        fs::remove_file(&path).unwrap();
        assert!(f.inspect().is_err());
        assert!(!path.exists());
    }

    #[test]
    #[cfg(unix)]
    fn linked_receipt_is_not_native_retirement_evidence() {
        let f = Fixture::new();
        retired(&f.host, &f.snapshot, &f.digest).unwrap();
        let path = f.snapshot.with_extension("retired");
        let other = f.host.root.join("other-receipt");
        fs::rename(&path, &other).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        assert!(f.inspect().is_err());
    }
}
