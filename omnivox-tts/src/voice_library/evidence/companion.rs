use super::{
    decode, path_text, read_bounded, require, sha256, Companion, FileIdentity, LibraryError,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;

pub(super) const MAX_FILES: usize = 8192;
const MAX_MANIFEST: usize = 2 * 1024 * 1024;

pub(super) fn relative_path(path: &str) -> Result<(), LibraryError> {
    require(
        !path.is_empty()
            && path.len() <= 1024
            && path.is_ascii()
            && !path.chars().any(char::is_control)
            && !path.contains(['\\', ':']),
        "unsafe companion path",
    )?;
    let parts: Vec<_> = path.split('/').collect();
    require(
        parts.len() <= 16
            && parts.iter().all(|part| {
                !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && !part.ends_with(['.', ' '])
                    && !matches!(
                        part.split('.')
                            .next()
                            .unwrap()
                            .to_ascii_uppercase()
                            .as_str(),
                        "CON"
                            | "PRN"
                            | "AUX"
                            | "NUL"
                            | "COM1"
                            | "COM2"
                            | "COM3"
                            | "COM4"
                            | "COM5"
                            | "COM6"
                            | "COM7"
                            | "COM8"
                            | "COM9"
                            | "LPT1"
                            | "LPT2"
                            | "LPT3"
                            | "LPT4"
                            | "LPT5"
                            | "LPT6"
                            | "LPT7"
                            | "LPT8"
                            | "LPT9"
                    )
            }),
        "unsafe companion path",
    )
}

fn ordinary(metadata: &Metadata) -> Result<(), LibraryError> {
    require(
        !metadata.file_type().is_symlink(),
        "companion links are unsupported",
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        require(
            metadata.file_attributes() & 0x400 == 0,
            "companion reparse points are unsupported",
        )?;
    }
    Ok(())
}

fn open_regular(path: &Path) -> Result<File, LibraryError> {
    let metadata = fs::symlink_metadata(path)?;
    ordinary(&metadata)?;
    require(metadata.is_file(), "evidence input is not a regular file")?;
    let file = File::open(path)?;
    require(
        file.metadata()?.is_file(),
        "opened evidence input is not a regular file",
    )?;
    Ok(file)
}

fn identity(bytes: &[u8]) -> FileIdentity {
    FileIdentity {
        bytes: bytes.len() as u64,
        sha256: hex(&Sha256::digest(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn file_identity(path: &Path) -> Result<FileIdentity, LibraryError> {
    let mut file = open_regular(path)?;
    let before = file.metadata()?;
    let mut remaining = before.len();
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let limit = remaining.min(buffer.len() as u64) as usize;
        let count = match file.read(&mut buffer[..limit]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        require(count > 0, "evidence input became shorter")?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
    }
    require(file.read(&mut [0])? == 0, "evidence input became longer")?;
    let after = file.metadata()?;
    require(
        before.len() == after.len() && before.modified().ok() == after.modified().ok(),
        "evidence input changed while reading",
    )?;
    Ok(FileIdentity {
        bytes: before.len(),
        sha256: hex(&hash.finalize()),
    })
}

fn inventory(bytes: &[u8]) -> Result<BTreeMap<String, String>, LibraryError> {
    let contents = std::str::from_utf8(bytes)
        .map_err(|_| LibraryError::Invalid("non-UTF-8 companion inventory"))?;
    require(contents.ends_with('\n'), "incomplete companion inventory")?;
    let mut result = BTreeMap::new();
    let mut folded = BTreeSet::new();
    for line in contents.lines() {
        let (digest, name) = line
            .split_once("  ")
            .ok_or(LibraryError::Invalid("malformed companion inventory"))?;
        sha256(digest)?;
        relative_path(name)?;
        require(
            name != "SHA256SUMS" && folded.insert(name.to_ascii_lowercase()),
            "duplicate or self-referencing companion inventory entry",
        )?;
        result.insert(name.into(), digest.into());
        require(
            result.len() < MAX_FILES,
            "companion inventory exceeds entry limit",
        )?;
    }
    Ok(result)
}

fn list(
    root: &Path,
    relative: &str,
    entries: &mut usize,
    files: &mut BTreeSet<String>,
) -> Result<(), LibraryError> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        *entries += 1;
        require(*entries <= MAX_FILES, "companion tree exceeds entry limit")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| LibraryError::Invalid("non-UTF-8 companion filename"))?;
        let name = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        relative_path(&name)?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ordinary(&metadata)?;
        if metadata.is_dir() {
            list(root, &name, entries, files)?;
        } else {
            require(metadata.is_file(), "unsupported companion file type")?;
            files.insert(name);
        }
    }
    Ok(())
}

fn native_target(target: &str) -> bool {
    // Helpers are separate executables: their compiler ABI need not equal the
    // supervisor's. Preserve Windows GNU/MSVC interoperability at this boundary.
    let suffixes: &[&str] = match std::env::consts::OS {
        "macos" => &["apple-darwin"],
        "windows" => &["pc-windows-msvc", "pc-windows-gnu"],
        "linux" => &["unknown-linux-gnu", "unknown-linux-musl"],
        _ => &[],
    };
    suffixes
        .iter()
        .any(|suffix| target == format!("{}-{suffix}", std::env::consts::ARCH))
}

pub(super) fn capture(helper: &Path, engine: &str) -> Result<Companion, LibraryError> {
    let helper = helper.canonicalize()?;
    if engine == "rhvoice" {
        let runtime = std::env::var_os("OMNIVOX_RHVOICE_LIBRARY")
            .map(std::path::PathBuf::from)
            .ok_or(LibraryError::Invalid(
                "RHVoice validation requires an explicit runtime library",
            ))?;
        require(runtime.is_absolute(), "RHVoice runtime must be absolute")?;
        let runtime = runtime.canonicalize()?;
        let identity = file_identity(&runtime)?;
        return Ok(Companion {
            helper: path_text(&helper)?,
            files: BTreeMap::from([("helper".into(), file_identity(&helper)?)]),
            external_runtime: Some(super::super::AssetFile {
                path: super::super::catalogue::metadata_path(&runtime)?,
                bytes: identity.bytes,
                sha256: identity.sha256,
            }),
        });
    }
    let root = helper
        .parent()
        .ok_or(LibraryError::Invalid("helper has no parent"))?;
    let expected_name = format!("omnivox-{engine}-helper{}", std::env::consts::EXE_SUFFIX);
    require(
        helper.file_name().and_then(|name| name.to_str()) == Some(&expected_name),
        "evidence requires a staged companion helper",
    )?;
    let manifest = read_bounded(open_regular(&root.join("SHA256SUMS"))?, MAX_MANIFEST)?;
    let expected = inventory(&manifest)?;
    let mut actual = BTreeSet::new();
    list(root, "", &mut 0, &mut actual)?;
    require(
        actual.remove("SHA256SUMS") && actual.iter().eq(expected.keys()),
        "companion inventory has missing or unlisted files",
    )?;
    let mut files = BTreeMap::new();
    for (name, digest) in expected {
        let file = file_identity(&root.join(&name))?;
        require(file.sha256 == digest, "companion checksum mismatch")?;
        files.insert(name, file);
    }
    files.insert("SHA256SUMS".into(), identity(&manifest));
    require(
        files.contains_key(&expected_name),
        "helper missing from companion inventory",
    )?;
    let provenance_bytes = read_bounded(
        open_regular(&root.join("SOURCE-PROVENANCE.json"))?,
        MAX_MANIFEST,
    )?;
    require(
        files.get("SOURCE-PROVENANCE.json") == Some(&identity(&provenance_bytes)),
        "companion provenance changed while reading",
    )?;
    let provenance: serde_json::Value = decode(&provenance_bytes, MAX_MANIFEST)?;
    require(
        provenance["schema_version"] == 1
            && provenance["target"].as_str().is_some_and(native_target)
            && provenance["artifact"].as_str().is_some_and(|artifact| {
                artifact.starts_with(&format!("omnivox-{engine}-companion-"))
            }),
        "unsupported companion provenance",
    )?;
    if engine == "piper" {
        require(
            files.contains_key("espeak-ng-data/phontab") && !files.contains_key("phontab"),
            "evidence requires bundled Piper phonemizer data",
        )?;
        let native = match std::env::consts::OS {
            "windows" => files.contains_key("piper.dll") && files.contains_key("onnxruntime.dll"),
            "macos" => {
                files.contains_key("libpiper.dylib")
                    && files.keys().any(|name| {
                        name.starts_with("libonnxruntime.")
                            && name.ends_with(".dylib")
                            && !name.contains('/')
                    })
            }
            _ => files.contains_key("libpiper.so") && files.contains_key("libonnxruntime.so.1"),
        };
        require(native, "evidence requires bundled Piper native libraries")?;
    }
    Ok(Companion {
        external_runtime: None,
        helper: path_text(&helper)?,
        files,
    })
}
