//! Reconcile recorded cleanup, without acquiring authority over saved processes.
use super::execution::{EventDigest, MAX_EVENT_BYTES, MAX_WORKERS};
use super::storage::{new_file, open_file, ordinary};
use super::{
    decode, digest, read_bounded, require, required_nullable, text, Inspection, LibraryError,
    Operation, ValidationState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::{ErrorKind, Write};

const RECEIPT: &str = "cleanup-recovery.frames";
const MAX_RECEIPT_BYTES: usize = 128 * 1024;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    schema_version: u32,
    outcome: String,
    basis: String,
    operation_id: String,
    plan_sha256: String,
    journal_sha256: String,
    workers: Vec<EventDigest>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerEvent {
    schema_version: u32,
    operation_id: String,
    plan_sha256: String,
    worker: u32,
    phase: String,
    #[serde(deserialize_with = "required_nullable")]
    program: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    arguments: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    pid: Option<u32>,
    ownership: String,
    recovery_authority: String,
}

impl Recovery {
    fn observe(operation: &Operation) -> Result<Self, LibraryError> {
        require(
            operation.journal().damage().is_none()
                && operation.journal().state() == Some(ValidationState::Validating),
            "cleanup recovery requires an intact interrupted validation journal",
        )?;
        let plan = operation.plan();
        let directory = operation.path().join("workers");
        ordinary(&directory, false)?;
        let mut names = BTreeSet::new();
        for entry in fs::read_dir(&directory)? {
            require(
                names.len() < MAX_WORKERS as usize * 3,
                "worker history exceeds limit",
            )?;
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| LibraryError::Invalid("invalid worker filename"))?;
            names.insert(name);
        }
        // An empty, initialized directory is safe: no child may be spawned
        // before an intent file is completely written and synchronized.
        require(
            names.len().is_multiple_of(3)
                && names.len() / 3 <= plan.generation().validation_targets().len() + 2,
            "worker history is incomplete or exceeds the planned load count",
        )?;
        let mut workers = Vec::new();
        let plan_sha256 = plan.sha256();
        for worker in 0..names.len() / 3 {
            for phase in ["intent", "owned", "cleaned"] {
                let name = format!("{worker:04}-{phase}.json");
                require(
                    names.contains(&name),
                    "worker history has missing or unexpected events",
                )?;
                let bytes =
                    read_bounded(open_file(&directory.join(&name), false)?, MAX_EVENT_BYTES)?;
                let event: WorkerEvent = decode(&bytes, MAX_EVENT_BYTES)?;
                require(
                    event.schema_version == 1
                        && event.operation_id == plan.document().operation_id
                        && event.plan_sha256 == plan_sha256
                        && event.worker as usize == worker
                        && event.phase == phase
                        && event.ownership
                            == if plan.document().platform == "windows" {
                                "private-job"
                            } else {
                                "private-process-group"
                            }
                        && event.recovery_authority == "live-supervisor-only",
                    "worker observation identity or ownership mismatch",
                )?;
                match phase {
                    "intent" => {
                        let program = event
                            .program
                            .as_deref()
                            .ok_or(LibraryError::Invalid("worker intent has no program"))?;
                        let arguments = event
                            .arguments
                            .as_ref()
                            .ok_or(LibraryError::Invalid("worker intent has no arguments"))?;
                        text(program, 4096)?;
                        require(
                            arguments.len() <= 16 && event.pid.is_none(),
                            "invalid worker intent",
                        )?;
                        for argument in arguments {
                            text(argument, 4096)?;
                        }
                    }
                    "owned" => require(
                        event.program.is_none()
                            && event.arguments.is_none()
                            && event.pid.is_some_and(|pid| pid > 0),
                        "invalid worker owner observation",
                    )?,
                    "cleaned" => require(
                        event.program.is_none() && event.arguments.is_none() && event.pid.is_none(),
                        "invalid worker cleanup observation",
                    )?,
                    _ => unreachable!(),
                }
                workers.push(EventDigest {
                    name,
                    sha256: digest(&bytes),
                });
            }
        }
        Ok(Self {
            schema_version: 1,
            outcome: "abandoned".into(),
            basis: "completed-worker-records-v1".into(),
            operation_id: plan.document().operation_id.clone(),
            plan_sha256,
            journal_sha256: digest(operation.journal().source_bytes()),
            workers,
        })
    }
}

/// Verify any existing receipt against all original inputs on every reopening.
/// Missing means unreconciled; malformed or changed evidence blocks inspection.
pub(super) fn check(operation: &Operation) -> Result<bool, LibraryError> {
    let path = operation.path().join(RECEIPT);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
        Ok(_) => (),
    }
    let bytes = read_bounded(open_file(&path, false)?, MAX_RECEIPT_BYTES)?;
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let body = lines.next().unwrap();
    require(
        lines.next() == Some(digest(body).as_bytes())
            && lines.next() == Some(&[][..])
            && lines.next().is_none(),
        "incomplete or damaged cleanup recovery receipt",
    )?;
    let saved: Recovery = decode(body, MAX_RECEIPT_BYTES)?;
    require(
        saved == Recovery::observe(operation)?,
        "cleanup recovery evidence changed",
    )?;
    Ok(true)
}

pub(super) fn abandon(operation: &Operation) -> Result<(), LibraryError> {
    require(
        operation.inspection() == Inspection::Interrupted,
        "only interrupted validation can be abandoned",
    )?;
    let recovery = Recovery::observe(operation)?;
    // Check the exact plan and journal again before adding a separate receipt.
    // Original bytes, including any saved report, are never replaced or promoted.
    operation.check_unchanged()?;
    let body = serde_json::to_vec(&recovery)?;
    let mut frame = body.clone();
    frame.push(b'\n');
    frame.extend_from_slice(digest(&body).as_bytes());
    frame.push(b'\n');
    require(
        frame.len() <= MAX_RECEIPT_BYTES,
        "cleanup recovery receipt exceeds limit",
    )?;
    let mut file = new_file(&operation.path().join(RECEIPT))?;
    file.write_all(&frame)?;
    file.sync_all()?;
    #[cfg(unix)]
    fs::File::open(operation.path())?.sync_all()?;
    require(
        check(operation)?,
        "cleanup recovery receipt was not retained",
    )
}
