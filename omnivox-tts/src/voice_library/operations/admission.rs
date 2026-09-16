//! Profile admission for cooperating managers using one provider-selected root.
//! Claims are permanent. Terminal results or verified abandonment release them.
use super::storage::{new_file, open_file, ordinary};
use super::{
    decode, digest, read_bounded, require, uuid, Inspection, LibraryError, Operation, Transition,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_CLAIMS: usize = 4096;
const MAX_METADATA_BYTES: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    schema_version: u32,
    target_id: String,
    profile_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    schema_version: u32,
    target_id: String,
    profile_id: String,
    operation_id: String,
    plan_sha256: String,
}

#[derive(Debug)]
pub struct AdmissionEntry {
    pub operation_id: String,
    pub state: Inspection,
}

/// Holds a profile lease. Provider/root provisioning is a separate operation.
pub struct Admission {
    path: PathBuf,
    operations: PathBuf,
    lease: File,
    identity: Identity,
    identity_bytes: Vec<u8>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        // Release even if an unrelated Unix fork temporarily copied this fd.
        let _ = self.lease.unlock();
    }
}

/// The profile lease must outlive the admitted operation and all native work.
pub struct AdmittedOperation<'a> {
    _admission: &'a mut Admission,
    operation: Operation,
}
impl AdmittedOperation<'_> {
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
    pub fn append(&mut self, transition: Transition) -> Result<(), LibraryError> {
        self.operation.append(transition)
    }
}

impl Admission {
    /// Initialize a new gate below an existing profiles/PROFILE_UUID directory.
    /// ROOT/operations must also exist. No existing or partial gate is replaced.
    pub fn create(root: &Path, target: &str, profile: &str) -> Result<Self, LibraryError> {
        uuid(target)?;
        let (path, operations) = paths(root, profile)?;
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder.create(&path)?;
        let lease = new_file(&path.join("owner.lock"))?;
        lease.try_lock().map_err(lock_error)?;
        lease.sync_all()?;
        builder.create(path.join("claims"))?;
        let identity = Identity {
            schema_version: 1,
            target_id: target.into(),
            profile_id: profile.into(),
        };
        let identity_bytes = serde_json::to_vec(&identity)?;
        let mut file = new_file(&path.join("identity.json"))?;
        file.write_all(&identity_bytes)?;
        file.sync_all()?;
        sync_directory(&path.join("claims"))?;
        sync_directory(&path)?;
        sync_directory(path.parent().unwrap())?;
        Ok(Self {
            path,
            operations,
            lease,
            identity,
            identity_bytes,
        })
    }

    /// Acquire a gate without waiting or repairing missing initialization.
    pub fn try_open(root: &Path, profile: &str) -> Result<Option<Self>, LibraryError> {
        let (path, operations) = paths(root, profile)?;
        ordinary(&path, false)?;
        // Creation takes the lease before publishing identity.json.
        ordinary(&path.join("identity.json"), true)?;
        let lease = open_file(&path.join("owner.lock"), true)?;
        match lease.try_lock() {
            Ok(()) => (),
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(error) => return Err(lock_error(error)),
        }
        ordinary(&path.join("claims"), false)?;
        let identity_bytes = read_bounded(
            open_file(&path.join("identity.json"), false)?,
            MAX_METADATA_BYTES,
        )?;
        let identity: Identity = decode(&identity_bytes, MAX_METADATA_BYTES)?;
        require(
            identity.schema_version == 1 && identity.profile_id == profile,
            "admission identity mismatch",
        )?;
        uuid(&identity.target_id)?;
        Ok(Some(Self {
            path,
            operations,
            lease,
            identity,
            identity_bytes,
        }))
    }

    pub fn target_id(&self) -> &str {
        &self.identity.target_id
    }

    /// Retain the operation lease while consuming one claimed native result.
    /// This is installation evidence, never authority to restart old work.
    pub(in crate::voice_library) fn completed_validation(
        &self,
        operation_id: &str,
    ) -> Result<Operation, LibraryError> {
        uuid(operation_id)?;
        self.check_identity()?;
        let claims = self.claims()?;
        let claim = claims
            .iter()
            .find(|claim| claim.operation_id == operation_id)
            .ok_or(LibraryError::Invalid(
                "validation has no profile admission claim",
            ))?;
        let operation = Operation::try_open(&self.operations.join(operation_id))?.ok_or(
            LibraryError::Invalid("validation operation is already owned"),
        )?;
        self.check_operation(&operation)?;
        require(
            operation.plan().sha256() == claim.plan_sha256,
            "claimed validation plan changed",
        )?;
        require(
            operation.inspection() == Inspection::Staged,
            "installation requires successful staged validation",
        )?;
        Ok(operation)
    }

    /// Read all claimed operations under the profile lock. Missing, changed or
    /// malformed history is an error, never evidence that the profile is clear.
    pub fn inspect(&self) -> Result<Vec<AdmissionEntry>, LibraryError> {
        self.check_identity()?;
        let mut result = Vec::new();
        for claim in self.claims()? {
            let state = match Operation::try_open(&self.operations.join(&claim.operation_id))? {
                None => Inspection::Busy,
                Some(operation) => {
                    self.check_operation(&operation)?;
                    require(
                        operation.plan().sha256() == claim.plan_sha256,
                        "claimed plan changed",
                    )?;
                    operation.inspection()
                }
            };
            result.push(AdmissionEntry {
                operation_id: claim.operation_id,
                state,
            });
        }
        Ok(result)
    }

    /// Claim a prepared operation, retaining the profile lock in the returned
    /// borrow. A different pending operation blocks admission, even if its owner
    /// has died. The same prepared operation can resume before native startup.
    pub fn admit(&mut self, operation_id: &str) -> Result<AdmittedOperation<'_>, LibraryError> {
        uuid(operation_id)?;
        self.check_identity()?;
        let claims = self.claims()?;
        for entry in self.inspect()? {
            require(
                matches!(
                    entry.state,
                    Inspection::Staged
                        | Inspection::Cancelled
                        | Inspection::Failed
                        | Inspection::Abandoned
                ) || (entry.operation_id == operation_id && entry.state == Inspection::Prepared),
                "profile has an unresolved validation operation; inspect retained history",
            )?;
        }
        let operation = Operation::try_open(&self.operations.join(operation_id))?.ok_or(
            LibraryError::Invalid("validation operation is already owned"),
        )?;
        self.check_operation(&operation)?;
        require(
            operation.inspection() == Inspection::Prepared,
            "only a prepared operation can be admitted",
        )?;
        if let Some(claim) = claims
            .iter()
            .find(|claim| claim.operation_id == operation_id)
        {
            require(
                claim.plan_sha256 == operation.plan().sha256(),
                "claimed plan changed",
            )?;
        } else {
            require(
                claims.len() < MAX_CLAIMS,
                "profile admission history is full; retention review required",
            )?;
            let claim = Claim {
                schema_version: 1,
                target_id: self.identity.target_id.clone(),
                profile_id: self.identity.profile_id.clone(),
                operation_id: operation_id.into(),
                plan_sha256: operation.plan().sha256(),
            };
            let body = serde_json::to_vec(&claim)?;
            let mut frame = body.clone();
            frame.push(b'\n');
            frame.extend_from_slice(digest(&body).as_bytes());
            frame.push(b'\n');
            let path = self.path.join("claims").join(operation_id);
            let mut file = new_file(&path)?;
            file.write_all(&frame)?;
            file.sync_all()?;
            sync_directory(path.parent().unwrap())?;
        }
        Ok(AdmittedOperation {
            _admission: self,
            operation,
        })
    }

    /// Abandon an interrupted validation only when every retained worker has a
    /// complete cleanup record and the journal has a verified validating prefix.
    /// A damaged suffix is retained. Holds both leases, preserves the journal, and
    /// never signals processes or promotes a saved report to validation success.
    /// An already verified abandonment is an idempotent success.
    pub fn abandon_cleaned_validation(&mut self, operation_id: &str) -> Result<(), LibraryError> {
        uuid(operation_id)?;
        self.check_identity()?;
        let claims = self.claims()?;
        let claim = claims
            .iter()
            .find(|claim| claim.operation_id == operation_id)
            .ok_or(LibraryError::Invalid(
                "operation has no profile admission claim",
            ))?;
        let operation = Operation::try_open(&self.operations.join(operation_id))?.ok_or(
            LibraryError::Invalid("validation operation is already owned"),
        )?;
        self.check_operation(&operation)?;
        require(
            operation.plan().sha256() == claim.plan_sha256,
            "claimed plan changed",
        )?;
        if operation.inspection() == Inspection::Abandoned {
            return Ok(());
        }
        super::recovery::abandon(&operation)
    }

    fn check_identity(&self) -> Result<(), LibraryError> {
        let bytes = read_bounded(
            open_file(&self.path.join("identity.json"), false)?,
            MAX_METADATA_BYTES,
        )?;
        require(
            bytes == self.identity_bytes,
            "profile admission identity changed",
        )
    }
    fn check_operation(&self, operation: &Operation) -> Result<(), LibraryError> {
        let generation = operation.plan().generation().document();
        require(
            generation.target_id == self.identity.target_id
                && generation.profile_id == self.identity.profile_id,
            "operation target/profile differs from admission owner",
        )
    }
    fn claims(&self) -> Result<Vec<Claim>, LibraryError> {
        let directory = self.path.join("claims");
        ordinary(&directory, false)?;
        let mut claims = Vec::new();
        for entry in fs::read_dir(directory)? {
            require(
                claims.len() < MAX_CLAIMS,
                "profile admission history exceeds limit",
            )?;
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or(LibraryError::Invalid("invalid admission filename"))?;
            uuid(name)?;
            let bytes = read_bounded(open_file(&entry.path(), false)?, MAX_METADATA_BYTES)?;
            let mut lines = bytes.split(|byte| *byte == b'\n');
            let body = lines.next().unwrap();
            require(
                lines.next() == Some(digest(body).as_bytes())
                    && lines.next() == Some(&[][..])
                    && lines.next().is_none(),
                "incomplete or damaged admission claim",
            )?;
            let claim: Claim = decode(body, MAX_METADATA_BYTES)?;
            require(
                claim.schema_version == 1
                    && claim.operation_id == name
                    && claim.target_id == self.identity.target_id
                    && claim.profile_id == self.identity.profile_id,
                "admission claim identity mismatch",
            )?;
            super::sha256(&claim.plan_sha256)?;
            claims.push(claim);
        }
        claims.sort_by(|a, b| a.operation_id.cmp(&b.operation_id));
        Ok(claims)
    }
}

fn paths(root: &Path, profile: &str) -> Result<(PathBuf, PathBuf), LibraryError> {
    uuid(profile)?;
    let root = root.canonicalize()?;
    let profiles = root.join("profiles");
    let directory = profiles.join(profile);
    let operations = root.join("operations");
    ordinary(&profiles, false)?;
    ordinary(&directory, false)?;
    ordinary(&operations, false)?;
    Ok((directory.join("validation-admission"), operations))
}
fn lock_error(error: TryLockError) -> LibraryError {
    match error {
        TryLockError::WouldBlock => LibraryError::Invalid("profile validation is already owned"),
        TryLockError::Error(error) => LibraryError::Io(error),
    }
}
fn sync_directory(path: &Path) -> Result<(), LibraryError> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
