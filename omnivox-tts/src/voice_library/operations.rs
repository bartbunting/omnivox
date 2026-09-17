//! Development validation-operation records. No installation or activation.
//! Journal statements record a trusted supervisor's observations, not independent
//! proof of cleanup. Reopening an unfinished native operation never grants reuse.
use super::verification::digest;
use super::{
    decode, native_path, read_bounded, require, required_nullable, sha256, text, uuid,
    HostPlatform, LibraryError, RuntimeLibrary,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

pub(super) mod storage;
pub use storage::{Inspection, Operation};
mod admission;
pub use admission::{Admission, AdmissionEntry, AdmittedOperation};
mod execution;
pub use execution::{BoundValidationEvidence, ExecutionRecords};
mod recovery;
#[cfg(test)]
mod tests;

pub const MAX_PLAN_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RECORD_BYTES: usize = 8 * 1024;
pub const MAX_RECORDS: usize = 1024;
pub const MAX_JOURNAL_BYTES: usize = MAX_RECORDS * (MAX_RECORD_BYTES + 66);

/// A frozen validation request, not a complete installation/activation plan.
/// Paths are metadata here; a later admission provider must recheck the actual
/// target, runtime, environment and profile ownership before starting work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationPlanDocument {
    pub schema_version: u32,
    pub operation_kind: String,
    pub operation_id: String,
    pub platform: String,
    pub generation_json: String,
    pub validator_path: String,
    pub helpers: BTreeMap<String, String>,
    pub timeout_seconds: u64,
    pub memory_bytes: u64,
    pub runtime_policy: String,
}

#[derive(Debug, Clone)]
pub struct ValidationPlan {
    document: ValidationPlanDocument,
    generation: RuntimeLibrary,
    source: Vec<u8>,
}

impl ValidationPlan {
    pub fn parse(bytes: &[u8]) -> Result<Self, LibraryError> {
        let document: ValidationPlanDocument = decode(bytes, MAX_PLAN_BYTES)?;
        require(
            document.schema_version == 1,
            "unknown validation plan schema",
        )?;
        require(
            document.operation_kind == "native_validation",
            "unsupported operation kind",
        )?;
        uuid(&document.operation_id)?;
        let host = match document.platform.as_str() {
            "windows" => HostPlatform::Windows,
            "linux" | "macos" => HostPlatform::Posix,
            _ => {
                return Err(LibraryError::Invalid(
                    "unsupported validation plan platform",
                ))
            }
        };
        let generation = RuntimeLibrary::parse(document.generation_json.as_bytes(), host)?;
        native_path(&document.validator_path, host)?;
        let engines: BTreeSet<_> = generation
            .validation_targets()
            .into_iter()
            .map(|voice| voice.engine_id)
            .collect();
        require(
            document.helpers.keys().eq(engines.iter()),
            "validation plan helper set differs from native load set",
        )?;
        for path in document.helpers.values() {
            native_path(path, host)?;
        }
        require(
            (1..=600).contains(&document.timeout_seconds),
            "invalid validation plan deadline",
        )?;
        require(
            (256 * 1024 * 1024..=65536 * 1024 * 1024).contains(&document.memory_bytes),
            "invalid validation plan memory budget",
        )?;
        require(
            document.runtime_policy
                == if engines.contains("rhvoice") {
                    "rhvoice-external-v1"
                } else {
                    "bundled-companions-v1"
                },
            "unknown validation runtime policy",
        )?;
        Ok(Self {
            document,
            generation,
            source: bytes.to_vec(),
        })
    }
    pub fn read(reader: impl Read) -> Result<Self, LibraryError> {
        Self::parse(&read_bounded(reader, MAX_PLAN_BYTES)?)
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }
    pub fn sha256(&self) -> String {
        digest(&self.source)
    }
    pub fn document(&self) -> &ValidationPlanDocument {
        &self.document
    }
    pub fn generation(&self) -> &RuntimeLibrary {
        &self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationState {
    Prepared,
    Validating,
    Staged,
    Cancelled,
    Failed,
    RecoveryFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cleanup {
    NotStarted,
    Unconfirmed,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub state: ValidationState,
    pub cleanup: Cleanup,
    #[serde(deserialize_with = "required_nullable")]
    pub evidence_sha256: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub detail: Option<String>,
}

impl Transition {
    fn validate(&self, previous: Option<ValidationState>) -> Result<(), LibraryError> {
        use ValidationState::*;
        let valid = matches!(
            (previous, self.state),
            (None, Prepared)
                | (Some(Prepared), Validating | Cancelled | Failed)
                | (
                    Some(Validating),
                    Staged | Cancelled | Failed | RecoveryFailed
                )
        );
        require(valid, "invalid validation operation state transition")?;
        let cleanup = match self.state {
            Prepared => Cleanup::NotStarted,
            Validating | RecoveryFailed => Cleanup::Unconfirmed,
            Cancelled | Failed if previous == Some(Prepared) => Cleanup::NotStarted,
            _ => Cleanup::Confirmed,
        };
        require(
            self.cleanup == cleanup,
            "operation cleanup statement contradicts its state",
        )?;
        require(
            self.evidence_sha256.is_some() == (self.state == Staged),
            "staged validation requires exactly one evidence digest",
        )?;
        if let Some(value) = &self.evidence_sha256 {
            sha256(value)?;
        }
        require(
            self.detail.is_some() == matches!(self.state, Cancelled | Failed | RecoveryFailed),
            "operation outcome requires a bounded explanation",
        )?;
        if let Some(value) = &self.detail {
            text(value, 1024)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub schema_version: u32,
    pub sequence: u32,
    pub previous_sha256: String,
    pub plan_sha256: String,
    /// Diagnostics only. Never signal or claim ownership using this PID.
    pub writer_pid: u32,
    pub recorded_unix_seconds: u64,
    pub transition: Transition,
}

/// All complete, consistent records up to the first damaged frame. A damaged
/// suffix is never removed or treated as a committed terminal state.
#[derive(Debug, Clone)]
pub struct Journal {
    records: Vec<JournalRecord>,
    digest: String,
    bytes: Vec<u8>,
    damage: Option<String>,
}

impl Journal {
    pub fn read(reader: impl Read, plan: &ValidationPlan) -> Result<Self, LibraryError> {
        let bytes = read_bounded(reader, MAX_JOURNAL_BYTES)?;
        let plan_sha256 = plan.sha256();
        let mut journal = Self {
            records: Vec::new(),
            digest: plan_sha256.clone(),
            bytes,
            damage: None,
        };
        let mut position = 0;
        while position < journal.bytes.len() {
            let parsed = journal.read_record(position, &plan_sha256);
            match parsed {
                Ok((record, checksum, next)) => {
                    journal.records.push(record);
                    journal.digest = checksum;
                    position = next;
                }
                Err(error) => {
                    journal.damage = Some(error.to_string());
                    break;
                }
            }
        }
        if journal.records.is_empty() && journal.damage.is_none() {
            journal.damage = Some("operation journal has no prepared record".into());
        }
        Ok(journal)
    }
    fn read_record(
        &self,
        position: usize,
        plan_sha256: &str,
    ) -> Result<(JournalRecord, String, usize), LibraryError> {
        require(
            self.records.len() < MAX_RECORDS,
            "operation journal has too many records",
        )?;
        let rest = &self.bytes[position..];
        let length = rest
            .iter()
            .take(MAX_RECORD_BYTES + 1)
            .position(|byte| *byte == b'\n')
            .ok_or(LibraryError::Invalid(
                "incomplete or oversized journal record",
            ))?;
        require(
            length > 0 && length <= MAX_RECORD_BYTES && rest.len() >= length + 66,
            "incomplete journal checksum frame",
        )?;
        let checksum = std::str::from_utf8(&rest[length + 1..length + 65])
            .map_err(|_| LibraryError::Invalid("invalid journal checksum encoding"))?;
        require(
            rest[length + 65] == b'\n' && checksum == digest(&rest[..length]),
            "operation journal checksum mismatch",
        )?;
        let record: JournalRecord = decode(&rest[..length], MAX_RECORD_BYTES)?;
        require(
            record.schema_version == 1
                && record.sequence as usize == self.records.len()
                && record.previous_sha256 == self.digest
                && record.plan_sha256 == plan_sha256,
            "operation journal identity or sequence mismatch",
        )?;
        require(
            record.writer_pid != 0 && record.recorded_unix_seconds > 0,
            "invalid journal writer diagnostics",
        )?;
        record.transition.validate(self.state())?;
        Ok((record, checksum.into(), position + length + 66))
    }
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }
    pub fn state(&self) -> Option<ValidationState> {
        self.records.last().map(|record| record.transition.state)
    }
    pub fn damage(&self) -> Option<&str> {
        self.damage.as_deref()
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn next_frame(
        &self,
        plan: &ValidationPlan,
        transition: Transition,
    ) -> Result<Vec<u8>, LibraryError> {
        require(
            self.damage.is_none(),
            "damaged operation journal cannot be extended",
        )?;
        require(
            self.records.len() < MAX_RECORDS,
            "operation journal has too many records",
        )?;
        transition.validate(self.state())?;
        let record = JournalRecord {
            schema_version: 1,
            sequence: self.records.len() as u32,
            previous_sha256: self.digest.clone(),
            plan_sha256: plan.sha256(),
            writer_pid: std::process::id(),
            recorded_unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| LibraryError::Invalid("system clock precedes Unix epoch"))?
                .as_secs(),
            transition,
        };
        let body = serde_json::to_vec(&record)?;
        require(
            body.len() <= MAX_RECORD_BYTES,
            "operation record exceeds byte limit",
        )?;
        let mut frame = body.clone();
        frame.push(b'\n');
        frame.extend_from_slice(digest(&body).as_bytes());
        frame.push(b'\n');
        Ok(frame)
    }
}
