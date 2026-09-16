//! Explicit filesystem ownership for one operation, not a profile admission lock.
use super::{Cleanup, Journal, LibraryError, Transition, ValidationPlan, ValidationState};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspection {
    Busy,
    Prepared,
    Interrupted,
    Staged,
    Cancelled,
    Failed,
    RecoveryFailed,
    Damaged,
}

/// Owns an OS lock on the permanent owner.lock file, private across exec.
/// Never unlink, rename or replace that file: that creates a second lock domain.
pub struct Operation {
    path: PathBuf,
    _lease: File,
    file: File,
    plan: ValidationPlan,
    journal: Journal,
    writable: bool,
    poisoned: bool,
}

impl Drop for Operation {
    fn drop(&mut self) {
        // A concurrent Unix fork may temporarily retain the descriptor until
        // exec. Release explicitly so normal retirement does not wait for that
        // unrelated child. Closing the file remains the fallback on error.
        let _ = self._lease.unlock();
    }
}

impl Operation {
    /// Create operations/OPERATION_UUID below an existing caller-selected root.
    /// Partial initialization is retained on failure; existing paths are untouched.
    pub fn create(parent: &Path, plan: ValidationPlan) -> Result<Self, LibraryError> {
        let parent = parent.canonicalize()?;
        let path = parent.join(&plan.document().operation_id);
        let directory = fs::DirBuilder::new();
        #[cfg(unix)]
        let directory = {
            use std::os::unix::fs::DirBuilderExt;
            let mut directory = directory;
            directory.mode(0o700);
            directory
        };
        directory.create(&path)?;
        let lease = new_file(&path.join("owner.lock"))?;
        lease.try_lock().map_err(lock_error)?;
        lease.sync_all()?;
        let mut plan_file = new_file(&path.join("plan.json"))?;
        plan_file.write_all(plan.source_bytes())?;
        plan_file.sync_all()?;
        let file = new_file(&path.join("journal.frames"))?;
        let journal = Journal {
            records: Vec::new(),
            digest: plan.sha256(),
            bytes: Vec::new(),
            damage: None,
        };
        let mut operation = Self {
            path,
            _lease: lease,
            file,
            plan,
            journal,
            writable: true,
            poisoned: false,
        };
        operation.write_transition(Transition {
            state: ValidationState::Prepared,
            cleanup: Cleanup::NotStarted,
            evidence_sha256: None,
            detail: None,
        })?;
        #[cfg(unix)]
        {
            File::open(&operation.path)?.sync_all()?;
            File::open(parent)?.sync_all()?;
        }
        Ok(operation)
    }

    /// Acquire an existing operation without waiting. None means an owner still
    /// holds its lock. Only an intact prepared operation may resume writing.
    pub fn try_open(path: &Path) -> Result<Option<Self>, LibraryError> {
        ordinary(path, false)?;
        let path = path.canonicalize()?;
        // The creator acquires its lease before creating plan.json. An inspector
        // must not take that lease in the earlier initialization window.
        ordinary(&path.join("plan.json"), true)?;
        let lease = open_file(&path.join("owner.lock"), true)?;
        match lease.try_lock() {
            Ok(()) => (),
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(error) => return Err(lock_error(error)),
        }
        let plan = ValidationPlan::read(open_file(&path.join("plan.json"), false)?)?;
        super::require(
            path.file_name().and_then(|name| name.to_str()) == Some(&plan.document().operation_id),
            "operation directory and plan identities differ",
        )?;
        let mut file = open_file(&path.join("journal.frames"), true)?;
        let journal = Journal::read(&mut file, &plan)?;
        let writable =
            journal.damage().is_none() && journal.state() == Some(ValidationState::Prepared);
        Ok(Some(Self {
            path,
            _lease: lease,
            file,
            plan,
            journal,
            writable,
            poisoned: false,
        }))
    }

    /// Inspect stable records under a temporary lease. No file is created or
    /// changed, and no saved PID is signalled. Malformed initialization is an error.
    pub fn inspect(path: &Path) -> Result<Inspection, LibraryError> {
        Ok(Self::try_open(path)?.map_or(Inspection::Busy, |operation| operation.inspection()))
    }
    fn inspection(&self) -> Inspection {
        if self.poisoned || self.journal.damage().is_some() {
            return Inspection::Damaged;
        }
        match self.journal.state() {
            Some(ValidationState::Prepared) => Inspection::Prepared,
            Some(ValidationState::Validating) => Inspection::Interrupted,
            Some(ValidationState::Staged) => Inspection::Staged,
            Some(ValidationState::Cancelled) => Inspection::Cancelled,
            Some(ValidationState::Failed) => Inspection::Failed,
            Some(ValidationState::RecoveryFailed) => Inspection::RecoveryFailed,
            None => Inspection::Damaged,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn plan(&self) -> &ValidationPlan {
        &self.plan
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Append and synchronize a supervisor statement while retaining ownership.
    /// The caller must record validating before native work and confirm cleanup
    /// before a terminal result. This layer does not verify that external work.
    /// Reclaimed validating/damaged/terminal operations cannot append at all.
    pub fn append(&mut self, transition: Transition) -> Result<(), LibraryError> {
        super::require(
            self.writable && !self.poisoned,
            "operation requires recovery or is already terminal",
        )?;
        let plan = ValidationPlan::read(open_file(&self.path.join("plan.json"), false)?)?;
        super::require(
            plan.source_bytes() == self.plan.source_bytes(),
            "operation plan changed",
        )?;
        let mut file = open_file(&self.path.join("journal.frames"), true)?;
        let current = Journal::read(&mut file, &self.plan)?;
        super::require(
            current.source_bytes() == self.journal.source_bytes(),
            "operation journal changed outside its owner",
        )?;
        self.file = file;
        self.write_transition(transition)
    }
    fn write_transition(&mut self, transition: Transition) -> Result<(), LibraryError> {
        let frame = self.journal.next_frame(&self.plan, transition)?;
        self.poisoned = true;
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&frame)?;
        self.file.sync_all()?;
        let mut bytes = self.journal.source_bytes().to_vec();
        bytes.extend_from_slice(&frame);
        self.journal = Journal::read(bytes.as_slice(), &self.plan)?;
        super::require(
            self.journal.damage().is_none(),
            "written operation record failed verification",
        )?;
        self.poisoned = false;
        Ok(())
    }
}

fn lock_error(error: TryLockError) -> LibraryError {
    match error {
        TryLockError::WouldBlock => LibraryError::Invalid("operation already has an owner"),
        TryLockError::Error(error) => LibraryError::Io(error),
    }
}

fn ordinary(path: &Path, file: bool) -> Result<(), LibraryError> {
    let metadata = fs::symlink_metadata(path)?;
    super::require(
        !metadata.file_type().is_symlink()
            && if file {
                metadata.is_file()
            } else {
                metadata.is_dir()
            },
        "operation path has an unsupported file type",
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        super::require(
            metadata.file_attributes() & 0x400 == 0,
            "operation reparse points are unsupported",
        )?;
    }
    Ok(())
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn new_file(path: &Path) -> Result<File, LibraryError> {
    Ok(options().create_new(true).open(path)?)
}
fn open_file(path: &Path, write: bool) -> Result<File, LibraryError> {
    ordinary(path, true)?;
    let file = OpenOptions::new().read(true).write(write).open(path)?;
    super::require(
        file.metadata()?.is_file(),
        "opened operation path is not a regular file",
    )?;
    Ok(file)
}
