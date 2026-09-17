//! Serialize package cleanup with publication of native startup references.
//! Missing retirement evidence retains a snapshot, including older owners.
use super::operations::storage::{new_file, open_file, ordinary};
use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Gate(File);
impl Gate {
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
    let bytes = read_bounded(open_file(snapshot, false)?, MAX_RUNTIME_BYTES)?;
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
