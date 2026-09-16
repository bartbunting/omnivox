//! Retained supervisor observations. Saved PIDs never authorize recovery signals.
use super::storage::{new_file, open_file, ordinary};
use super::{decode, digest, read_bounded, require, text, LibraryError, ValidationPlan};
use crate::voice_library::evidence::ValidationEvidence;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_WORKERS: u32 = 256;
const MAX_EVENT_BYTES: usize = 128 * 1024;
const MAX_BOUND_EVIDENCE_BYTES: usize = 18 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventDigest {
    name: String,
    sha256: String,
}

#[derive(Serialize)]
struct WorkerEvent<'a> {
    schema_version: u32,
    operation_id: &'a str,
    plan_sha256: &'a str,
    worker: u32,
    phase: &'a str,
    program: Option<&'a str>,
    arguments: Option<&'a [String]>,
    pid: Option<u32>,
    ownership: &'a str,
    recovery_authority: &'a str,
}

/// Held by the live admitted supervisor. No constructor resumes old records.
pub struct ExecutionRecords {
    path: PathBuf,
    operation_id: String,
    plan_sha256: String,
    worker: u32,
    pending: bool,
    owned: bool,
    events: Vec<EventDigest>,
}

impl ExecutionRecords {
    pub fn create(operation: &super::AdmittedOperation<'_>) -> Result<Self, LibraryError> {
        let operation = operation.operation();
        require(
            operation.journal().state() == Some(super::ValidationState::Validating),
            "execution requires a recorded validating operation",
        )?;
        let path = operation.path().join("workers");
        fs::create_dir(&path)?;
        sync_directory(operation.path())?;
        Ok(Self {
            path,
            operation_id: operation.plan().document().operation_id.clone(),
            plan_sha256: operation.plan().sha256(),
            worker: 0,
            pending: false,
            owned: false,
            events: Vec::new(),
        })
    }
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// Persist spawn intent before creating any child. An I/O failure blocks the
    /// operation even if the supervisor has not yet attempted process creation.
    pub fn starting(&mut self, program: &str, arguments: &[String]) -> Result<(), LibraryError> {
        require(
            !self.pending && self.worker < MAX_WORKERS,
            "previous worker is unresolved or worker limit reached",
        )?;
        text(program, 4096)?;
        require(
            arguments.len() <= 16,
            "too many validation worker arguments",
        )?;
        for argument in arguments {
            text(argument, 4096)?;
        }
        self.pending = true;
        self.event("intent", Some(program), Some(arguments), None)
    }
    /// Called only after the live supervisor assigns its private job/group and
    /// before START. The saved PID is diagnostic, not a boot/birth identity.
    pub fn owned(&mut self, pid: u32) -> Result<(), LibraryError> {
        require(
            self.pending && !self.owned && pid > 0,
            "invalid worker ownership observation",
        )?;
        self.event("owned", None, None, Some(pid))?;
        self.owned = true;
        Ok(())
    }
    /// Called only after process-tree and reader cleanup have both succeeded.
    pub fn cleaned(&mut self) -> Result<(), LibraryError> {
        require(
            self.pending && self.owned,
            "worker cleanup has no recorded owner",
        )?;
        self.event("cleaned", None, None, None)?;
        self.pending = false;
        self.owned = false;
        self.worker += 1;
        Ok(())
    }
    fn event(
        &mut self,
        phase: &str,
        program: Option<&str>,
        arguments: Option<&[String]>,
        pid: Option<u32>,
    ) -> Result<(), LibraryError> {
        let event = WorkerEvent {
            schema_version: 1,
            operation_id: &self.operation_id,
            plan_sha256: &self.plan_sha256,
            worker: self.worker,
            phase,
            program,
            arguments,
            pid,
            ownership: if cfg!(windows) {
                "private-job"
            } else {
                "private-process-group"
            },
            recovery_authority: "live-supervisor-only",
        };
        let bytes = serde_json::to_vec(&event)?;
        require(
            bytes.len() <= MAX_EVENT_BYTES,
            "worker event exceeds byte limit",
        )?;
        let name = format!("{:04}-{phase}.json", self.worker);
        let mut file = new_file(&self.path.join(&name))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        sync_directory(&self.path)?;
        self.events.push(EventDigest {
            name,
            sha256: digest(&bytes),
        });
        Ok(())
    }

    /// Bind this live attempt's complete observations to its exact plan. The
    /// supervisor must have performed the native checks; this is not attestation.
    pub fn bind(
        &self,
        plan: &ValidationPlan,
        evidence: &[u8],
    ) -> Result<BoundValidationEvidence, LibraryError> {
        require(
            !self.pending
                && self.worker as usize == plan.generation().validation_targets().len() + 2
                && plan.sha256() == self.plan_sha256,
            "validation workers are unresolved or belong to another plan",
        )?;
        ordinary(&self.path, false)?;
        for event in &self.events {
            let bytes = read_bounded(
                open_file(&self.path.join(&event.name), false)?,
                MAX_EVENT_BYTES,
            )?;
            require(
                digest(&bytes) == event.sha256,
                "recorded worker observation changed",
            )?;
        }
        let report = ValidationEvidence::read(evidence)?;
        require(
            report.matches_request(plan),
            "native evidence differs from the frozen request",
        )?;
        Ok(BoundValidationEvidence {
            schema_version: 1,
            operation_id: self.operation_id.clone(),
            plan_sha256: self.plan_sha256.clone(),
            evidence_json: String::from_utf8(evidence.to_vec())
                .map_err(|_| LibraryError::Invalid("non-UTF-8 validation evidence"))?,
            workers: self.events.clone(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundValidationEvidence {
    schema_version: u32,
    operation_id: String,
    plan_sha256: String,
    evidence_json: String,
    workers: Vec<EventDigest>,
}
impl BoundValidationEvidence {
    /// Read without opening any saved worker or runtime path. A report from
    /// another attempt cannot match merely because its generation is identical.
    pub fn read(reader: impl Read, plan: &ValidationPlan) -> Result<Self, LibraryError> {
        let result: Self = decode(
            &read_bounded(reader, MAX_BOUND_EVIDENCE_BYTES)?,
            MAX_BOUND_EVIDENCE_BYTES,
        )?;
        require(
            result.schema_version == 1
                && result.operation_id == plan.document().operation_id
                && result.plan_sha256 == plan.sha256(),
            "validation evidence belongs to another operation",
        )?;
        require(
            result.workers.len() == (plan.generation().validation_targets().len() + 2) * 3
                && result.workers.len() <= MAX_WORKERS as usize * 3
                && result.workers.len().is_multiple_of(3),
            "incomplete validation worker history",
        )?;
        for (index, event) in result.workers.iter().enumerate() {
            let phase = ["intent", "owned", "cleaned"][index % 3];
            require(
                event.name == format!("{:04}-{phase}.json", index / 3),
                "invalid validation worker ordering",
            )?;
            super::sha256(&event.sha256)?;
        }
        require(
            ValidationEvidence::read(result.evidence_json.as_bytes())?.matches_request(plan),
            "native evidence differs from the frozen request",
        )?;
        Ok(result)
    }
    pub fn save(&self, operation: &super::AdmittedOperation<'_>) -> Result<String, LibraryError> {
        let bytes = serde_json::to_vec(self)?;
        Self::read(bytes.as_slice(), operation.operation().plan())?;
        let directory = operation.operation().path();
        let mut file = new_file(&directory.join("validation-evidence.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        sync_directory(directory)?;
        Ok(digest(&bytes))
    }
}

fn sync_directory(path: &Path) -> Result<(), LibraryError> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
