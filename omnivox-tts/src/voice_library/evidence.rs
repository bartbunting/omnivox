//! Development evidence of observed inputs, never authority to skip validation.
//! Reading saved metadata does not open its paths. Capture must run in an owned,
//! deadline-limited worker: filesystem reads themselves can block.
use super::{
    decode, read_bounded, require, sha256, text, HostPlatform, LibraryError, RuntimeLibrary,
};
use crate::contracts::PhysicalVoiceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod companion;
#[cfg(test)]
mod tests;

pub const MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
const POLICY: &str = "managed-pcm-a-cleanup-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Companion {
    helper: String,
    files: BTreeMap<String, FileIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeLoad {
    projection_sha256: String,
    voices: Vec<PhysicalVoiceId>,
}

/// A bounded observation, including exact generation bytes and selected loads.
/// It does not attest mapped executable images, system libraries or publisher trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSnapshot {
    schema_version: u32,
    policy: String,
    os: String,
    arch: String,
    memory_policy: String,
    timeout_seconds: u64,
    memory_bytes: u64,
    working_directory: String,
    search_environment: BTreeMap<String, String>,
    generation_json: String,
    generation_sha256: String,
    loads: Vec<NativeLoad>,
    validator_path: String,
    validator: FileIdentity,
    companions: BTreeMap<String, Companion>,
}

/// A local, unauthenticated success observation. Construct only after all native
/// probes and cleanup succeed and a second snapshot equals the first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEvidence {
    schema_version: u32,
    kind: String,
    completed_unix_seconds: u64,
    cleanup_confirmed: bool,
    snapshot: EvidenceSnapshot,
}

fn host(os: &str) -> Result<HostPlatform, LibraryError> {
    match os {
        "windows" => Ok(HostPlatform::Windows),
        "linux" | "macos" => Ok(HostPlatform::Posix),
        _ => Err(LibraryError::Invalid("unsupported evidence platform")),
    }
}

fn memory_policy(os: &str) -> Result<&'static str, LibraryError> {
    match os {
        "linux" => Ok("inherited-address-space-v1"),
        "windows" => Ok("job-committed-memory-v1"),
        "macos" => Ok("sampled-group-footprint-v1"),
        _ => Err(LibraryError::Invalid("unsupported evidence platform")),
    }
}

fn loads(library: &RuntimeLibrary) -> Result<Vec<NativeLoad>, LibraryError> {
    library
        .validation_targets()
        .iter()
        .map(|target| {
            let unit = library.validation_unit(target)?;
            let voices = if target.engine_id == "piper" {
                unit.document().piper.as_ref().unwrap().models[0]
                    .voices
                    .iter()
                    .map(|voice| PhysicalVoiceId::new("piper", &voice.physical_id))
                    .collect()
            } else {
                vec![target.clone()]
            };
            Ok(NativeLoad {
                projection_sha256: unit.sha256(),
                voices,
            })
        })
        .collect()
}

impl EvidenceSnapshot {
    /// Observe caller-supplied inputs, verifying assets and complete staged
    /// companion inventories. The caller must reject runtime overrides first.
    pub fn capture(
        library: &RuntimeLibrary,
        validator: &Path,
        helpers: &BTreeMap<String, PathBuf>,
        timeout_seconds: u64,
        memory_bytes: u64,
    ) -> Result<Self, LibraryError> {
        library.verify_assets(super::ProviderOverrides::default())?;
        let mut companions = BTreeMap::new();
        for target in library.validation_targets() {
            if !companions.contains_key(&target.engine_id) {
                let helper = helpers
                    .get(&target.engine_id)
                    .ok_or(LibraryError::Invalid("missing evidence helper"))?;
                companions.insert(
                    target.engine_id.clone(),
                    companion::capture(helper, &target.engine_id)?,
                );
            }
        }
        let validator = validator.canonicalize()?;
        let snapshot = Self {
            schema_version: 1,
            policy: POLICY.into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            memory_policy: memory_policy(std::env::consts::OS)?.into(),
            timeout_seconds,
            memory_bytes,
            working_directory: path_text(&std::env::current_dir()?.canonicalize()?)?,
            search_environment: search_environment()?,
            generation_json: String::from_utf8(library.source_bytes().to_vec())
                .map_err(|_| LibraryError::Invalid("generation is not UTF-8"))?,
            generation_sha256: library.sha256(),
            loads: loads(library)?,
            validator_path: path_text(&validator)?,
            validator: companion::file_identity(&validator)?,
            companions,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Strict metadata-only reader, also used for private worker observations.
    pub fn read(reader: impl Read) -> Result<Self, LibraryError> {
        let snapshot: Self = decode(
            &read_bounded(reader, MAX_EVIDENCE_BYTES)?,
            MAX_EVIDENCE_BYTES,
        )?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, LibraryError> {
        encode(self)
    }

    fn validate(&self) -> Result<(), LibraryError> {
        require(
            self.schema_version == 1 && self.policy == POLICY,
            "unknown evidence policy",
        )?;
        text(&self.arch, 64)?;
        text(&self.working_directory, 4096)?;
        require(
            self.search_environment.len() <= 3,
            "invalid search environment",
        )?;
        for (name, value) in &self.search_environment {
            require(
                matches!(name.as_str(), "PATH" | "SystemRoot" | "WINDIR")
                    && value.len() <= 65536
                    && !value.contains('\0'),
                "invalid search environment",
            )?;
        }
        require(
            self.memory_policy == memory_policy(&self.os)?,
            "wrong memory policy",
        )?;
        require(
            (1..=600).contains(&self.timeout_seconds),
            "invalid evidence deadline",
        )?;
        require(
            (256 * 1024 * 1024..=65536 * 1024 * 1024).contains(&self.memory_bytes),
            "invalid evidence memory budget",
        )?;
        let library = RuntimeLibrary::parse(self.generation_json.as_bytes(), host(&self.os)?)?;
        require(
            self.generation_sha256 == library.sha256(),
            "evidence generation digest mismatch",
        )?;
        require(
            self.loads == loads(&library)?,
            "evidence native load set mismatch",
        )?;
        text(&self.validator_path, 4096)?;
        sha256(&self.validator.sha256)?;
        require(self.validator.bytes > 0, "empty validator")?;
        let engines: std::collections::BTreeSet<_> = library
            .validation_targets()
            .into_iter()
            .map(|target| target.engine_id)
            .collect();
        require(
            self.companions.keys().eq(engines.iter()),
            "evidence companion set mismatch",
        )?;
        for companion in self.companions.values() {
            text(&companion.helper, 4096)?;
            require(
                !companion.files.is_empty() && companion.files.len() <= companion::MAX_FILES,
                "invalid evidence companion inventory size",
            )?;
            for (path, file) in &companion.files {
                companion::relative_path(path)?;
                sha256(&file.sha256)?;
            }
            require(
                companion.files.contains_key("SHA256SUMS")
                    && companion.files.contains_key("SOURCE-PROVENANCE.json"),
                "missing evidence inventory or provenance",
            )?;
        }
        Ok(())
    }
}

impl ValidationEvidence {
    /// The supervisor supplies the time after its final input/cleanup checks.
    /// This constructor cannot establish that native validation actually ran.
    pub fn after_success(snapshot: EvidenceSnapshot, completed_unix_seconds: u64) -> Self {
        Self {
            schema_version: 1,
            kind: "native-validation-observation".into(),
            completed_unix_seconds,
            cleanup_confirmed: true,
            snapshot,
        }
    }

    /// Parse metadata without reading or executing any path stored in it.
    pub fn read(reader: impl Read) -> Result<Self, LibraryError> {
        let report: Self = decode(
            &read_bounded(reader, MAX_EVIDENCE_BYTES)?,
            MAX_EVIDENCE_BYTES,
        )?;
        require(
            report.schema_version == 1
                && report.kind == "native-validation-observation"
                && report.cleanup_confirmed
                && report.completed_unix_seconds > 0,
            "incomplete or unknown validation evidence",
        )?;
        report.snapshot.validate()?;
        Ok(report)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, LibraryError> {
        encode(self)
    }

    /// Equal observed inputs, not permission to reuse validation or activate.
    pub fn matches(&self, current: &EvidenceSnapshot) -> bool {
        self.snapshot == *current
    }
}

fn path_text(path: &Path) -> Result<String, LibraryError> {
    let value = path
        .to_str()
        .ok_or(LibraryError::Invalid("non-UTF-8 evidence path"))?;
    text(value, 4096)?;
    Ok(value.into())
}

fn search_environment() -> Result<BTreeMap<String, String>, LibraryError> {
    let mut result = BTreeMap::new();
    for name in ["PATH", "SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            result.insert(
                name.into(),
                value
                    .into_string()
                    .map_err(|_| LibraryError::Invalid("non-UTF-8 loader search environment"))?,
            );
        }
    }
    Ok(result)
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, LibraryError> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_EVIDENCE_BYTES - self.0.len() {
                return Err(std::io::Error::other("evidence exceeds byte limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, value)?;
    Ok(output.0)
}
