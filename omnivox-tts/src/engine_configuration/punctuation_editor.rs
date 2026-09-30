//! Local punctuation review and optimistic, atomic configuration edits.
//! No engines, speech state, client paths or remote management are involved.
use super::*;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Review {
    pub path: String,
    pub sha256: String,
    pub defaults: PunctuationTables,
    pub overrides: Value,
    pub effective: PunctuationTables,
    pub profiles: Value,
}

fn failure(reason: &'static str) -> ConfigurationError {
    ConfigurationError::new("configuration editor", reason)
}

fn canonical_destination(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| failure("invalid configuration root"))?;
            let name = path
                .file_name()
                .ok_or_else(|| failure("invalid configuration root"))?;
            Ok(canonical_destination(parent)?.join(name))
        }
        Err(_) => Err(failure("cannot resolve configuration root")),
    }
}

pub(super) fn path(root: &ConfigurationRoot) -> Result<PathBuf> {
    // Match ordinary startup validation, including explicitly missing roots.
    root.load()?;
    Ok(canonical_destination(&root.path)?.join("config.json"))
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(failure("cannot inspect configuration file")),
        Ok(metadata) if !metadata.is_file() || files::redirected(&metadata) => Err(failure(
            "editor requires an ordinary configuration file without links",
        )),
        Ok(_) => files::read_file(path, MAX_CONFIG_BYTES, true, &mut 0).map(Some),
    }
}

pub(super) fn revision(path: &Path, bytes: Option<&[u8]>) -> String {
    let mut hash = Sha256::new();
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update(if bytes.is_some() { [1] } else { [0] });
    hash.update(bytes.unwrap_or_default());
    hash.update(
        serde_json::to_vec(&PunctuationTables::default()).expect("shipped tables serialize"),
    );
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn document(bytes: Option<&[u8]>) -> Result<Value> {
    match bytes {
        Some(bytes) => {
            Configuration::parse(bytes, Platform::native())?;
            json::parse_configuration(bytes, MAX_CONFIG_BYTES)
        }
        None => Ok(serde_json::json!({"schema":3})),
    }
}

fn review(path: &Path, bytes: Option<&[u8]>) -> Result<Review> {
    let document = document(bytes)?;
    let configuration = Configuration::parse(
        &serde_json::to_vec(&document).map_err(|_| failure("cannot encode configuration"))?,
        Platform::native(),
    )?;
    Ok(Review {
        path: path.to_string_lossy().into_owned(),
        sha256: revision(path, bytes),
        defaults: PunctuationTables::default(),
        overrides: document
            .pointer("/speech/punctuation")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        effective: configuration.speech.punctuation,
        profiles: document
            .pointer("/speech/punctuation_profiles")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    })
}

pub fn inspect(root: &ConfigurationRoot) -> Result<Review> {
    let path = path(root)?;
    review(&path, read(&path)?.as_deref())
}

pub(super) fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    options
}

struct Lease(File);
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn save(root: &ConfigurationRoot, expected: &str, overrides_json: &[u8]) -> Result<Review> {
    save_profiles(root, expected, overrides_json, None)
}

pub fn save_profiles(
    root: &ConfigurationRoot,
    expected: &str,
    overrides_json: &[u8],
    profiles_json: Option<&[u8]>,
) -> Result<Review> {
    // Validate draft syntax and limits before making any filesystem changes.
    let overrides = json::parse_snapshot(overrides_json, MAX_CONFIG_BYTES)?;
    let draft = serde_json::json!({"schema":3,"speech":{"punctuation":overrides}});
    Configuration::parse(&serde_json::to_vec(&draft).unwrap(), Platform::native())?;
    let path = path(root)?;
    let original = read(&path)?;
    if revision(&path, original.as_deref()) != expected {
        return Err(failure(
            "file or target changed; refresh and review your edits",
        ));
    }
    let mut document = document(original.as_deref())?;
    let schema = document["schema"]
        .as_u64()
        .unwrap_or(3)
        .max(if profiles_json.is_some() { 4 } else { 3 });
    document["schema"] = Value::from(schema);
    let speech = document
        .as_object_mut()
        .unwrap()
        .entry("speech")
        .or_insert_with(|| serde_json::json!({}));
    speech
        .as_object_mut()
        .unwrap()
        .insert("punctuation".into(), overrides);
    if let Some(bytes) = profiles_json {
        let profiles = json::parse_snapshot(bytes, MAX_CONFIG_BYTES)?;
        speech
            .as_object_mut()
            .unwrap()
            .insert("punctuation_profiles".into(), profiles);
    }
    let mut bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| failure("cannot encode configuration"))?;
    bytes.push(b'\n');
    Configuration::parse(&bytes, Platform::native())?;
    publish(&path, original, &bytes)?;
    review(&path, Some(&bytes))
}

// Both editors use the same permanent lock and exact-byte conflict check.
pub(super) fn publish(path: &Path, original: Option<Vec<u8>>, bytes: &[u8]) -> Result<()> {
    publish_checked(path, original, bytes, || Ok(()))
}

pub(super) fn publish_checked(
    path: &Path,
    original: Option<Vec<u8>>,
    bytes: &[u8],
    validate: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let parent = path.parent().unwrap();
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory
        .create(parent)
        .map_err(|_| failure("cannot create configuration directory"))?;
    // A permanent lock file serializes editor writers across atomic renames.
    let lease = options()
        .create(true)
        .truncate(false)
        .open(parent.join(".punctuation.lock"))
        .map_err(|_| failure("cannot open punctuation editor lock"))?;
    let metadata = lease
        .metadata()
        .map_err(|_| failure("cannot inspect editor lock"))?;
    if !metadata.is_file() || files::redirected(&metadata) {
        return Err(failure("editor lock is not an ordinary file"));
    }
    lease
        .try_lock()
        .map_err(|_| failure("another configuration editor is saving; retry"))?;
    let _lease = Lease(lease);
    validate()?;
    if read(path)? != original {
        return Err(failure("file changed; refresh and review your edits"));
    }
    let id = crate::voice_library::local::new_uuid()
        .map_err(|_| failure("cannot create save identity"))?;
    let temporary = parent.join(format!(".punctuation-{id}.tmp"));
    let mut file = options()
        .create_new(true)
        .open(&temporary)
        .map_err(|_| failure("cannot prepare configuration save"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| failure("cannot write configuration"))?;
        if original.is_some() {
            let permissions = fs::metadata(path)
                .map_err(|_| failure("cannot inspect permissions"))?
                .permissions();
            file.set_permissions(permissions)
                .map_err(|_| failure("cannot preserve permissions"))?;
        }
        file.sync_all()
            .map_err(|_| failure("cannot sync configuration"))?;
        drop(file);
        if read(path)? != original {
            return Err(failure("file changed; refresh and review your edits"));
        }
        fs::rename(&temporary, path).map_err(|_| failure("cannot publish configuration"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| failure("save may have completed; refresh before retrying"))?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests;
