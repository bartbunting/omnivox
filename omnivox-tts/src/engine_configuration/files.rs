use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, Metadata, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use super::{
    shipped, Configuration, ConfigurationError, HelperManifest, Platform, Result, MAX_CONFIG_BYTES,
    MAX_MANIFESTS, MAX_MANIFEST_BYTES, MAX_MANIFEST_TOTAL_BYTES,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationRoot {
    pub path: PathBuf,
    pub explicit: bool,
}

#[derive(Clone)]
pub struct ManifestRegistration {
    pub source: PathBuf,
    pub manifest: HelperManifest,
}

#[derive(Clone, Default)]
pub struct LoadedConfiguration {
    pub root: Option<PathBuf>,
    pub configuration: Configuration,
    pub external: BTreeMap<String, ManifestRegistration>,
    /// Bounded source/field/reason diagnostics, never raw arguments or values.
    pub diagnostics: Vec<ConfigurationError>,
}

impl ConfigurationRoot {
    pub fn load(&self) -> Result<LoadedConfiguration> {
        Platform::native().validate_path(
            self.path
                .to_str()
                .ok_or_else(|| at(&self.path, "configuration root is not UTF-8"))?,
            "configuration root",
        )?;
        let root = match self.path.canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == ErrorKind::NotFound && !self.explicit => {
                // A dangling redirected root is an invalid existing root.
                match fs::symlink_metadata(&self.path) {
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        return Ok(LoadedConfiguration::default())
                    }
                    _ => return Err(at(&self.path, "configuration root cannot be resolved")),
                }
            }
            Err(_) => return Err(at(&self.path, "configuration root cannot be resolved")),
        };
        // Opening the iterator establishes readability even without any files.
        fs::read_dir(&root)
            .map_err(|_| at(&root, "configuration root is not a readable directory"))?;
        let policy_path = root.join("config.json");
        let configuration = match fs::symlink_metadata(&policy_path) {
            Ok(_) => {
                let bytes = read_file(&policy_path, MAX_CONFIG_BYTES, false, &mut 0)?;
                Configuration::parse(&bytes, Platform::native())
                    .map_err(|e| source(&policy_path, e))?
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Configuration::default(),
            Err(_) => return Err(at(&policy_path, "cannot inspect main configuration")),
        };
        let mut result = LoadedConfiguration {
            root: Some(root.clone()),
            configuration,
            ..LoadedConfiguration::default()
        };
        let directory = root.join("helpers.d");
        if let Err(error) = result.read_manifests(&directory) {
            result.external.clear();
            result.diagnostics.push(error);
        }
        result
            .configuration
            .validate_overrides(&result.external.keys().cloned().collect())
            .map_err(|e| source(&policy_path, e))?;
        Ok(result)
    }
}

impl LoadedConfiguration {
    fn read_manifests(&mut self, directory: &Path) -> Result<()> {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() && !redirected(&metadata) => (),
            Ok(_) => {
                return Err(at(
                    directory,
                    "helper directory must be an ordinary directory",
                ))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(at(directory, "cannot inspect helper directory")),
        }
        let entries =
            fs::read_dir(directory).map_err(|_| at(directory, "cannot read helper directory"))?;
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| at(directory, "cannot read helper directory entry"))?;
            let name = entry.file_name();
            // Non-UTF-8 JSON names cannot participate in the specified ordering.
            let Some(name) = name.to_str() else {
                if name.to_string_lossy().ends_with(".json") {
                    return Err(at(directory, "JSON manifest filename is not UTF-8"));
                }
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            if candidates.len() == MAX_MANIFESTS {
                return Err(at(directory, "too many candidate manifests"));
            }
            candidates.push((name.to_owned(), entry.path()));
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        let mut bytes_total = 0u64;
        let mut conflicts = BTreeSet::new();
        for (_, path) in candidates {
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !redirected(&metadata) => metadata,
                _ => {
                    self.diagnostics.push(at(
                        &path,
                        "manifest must be a direct regular file without redirection",
                    ));
                    continue;
                }
            };
            bytes_total = bytes_total.saturating_add(metadata.len());
            if bytes_total > MAX_MANIFEST_TOTAL_BYTES as u64 {
                return Err(at(directory, "aggregate manifest byte limit exceeded"));
            }
            if metadata.len() > MAX_MANIFEST_BYTES as u64 {
                self.diagnostics
                    .push(at(&path, "manifest exceeds byte limit"));
                continue;
            }
            let mut observed_bytes = metadata.len();
            let read = read_file(&path, MAX_MANIFEST_BYTES, true, &mut observed_bytes);
            // Failed or oversized reads still count toward the whole-set bound.
            bytes_total = bytes_total.saturating_add(observed_bytes.saturating_sub(metadata.len()));
            if bytes_total > MAX_MANIFEST_TOTAL_BYTES as u64 {
                return Err(at(directory, "aggregate manifest byte limit exceeded"));
            }
            let bytes = match read {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.diagnostics.push(error);
                    continue;
                }
            };
            let manifest = match HelperManifest::parse(&bytes, Platform::native()) {
                Ok(manifest) => manifest,
                Err(error) => {
                    self.diagnostics.push(source(&path, error));
                    continue;
                }
            };
            let id = &manifest.engine_id;
            if shipped::reserved(id) {
                self.diagnostics.push(at(
                    &path,
                    "engine ID is reserved by a shipped engine or alias",
                ));
            } else if conflicts.contains(id) {
                self.diagnostics
                    .push(at(&path, "duplicate external engine ID"));
            } else if let Some(previous) = self.external.remove(id) {
                conflicts.insert(id.clone());
                self.diagnostics
                    .push(at(&previous.source, "duplicate external engine ID"));
                self.diagnostics
                    .push(at(&path, "duplicate external engine ID"));
            } else {
                self.external.insert(
                    id.clone(),
                    ManifestRegistration {
                        source: path,
                        manifest,
                    },
                );
            }
        }
        Ok(())
    }
}

fn source(path: &Path, error: ConfigurationError) -> ConfigurationError {
    ConfigurationError::new(format!("{}: {}", path.display(), error.field), error.reason)
}

fn at(path: &Path, reason: &'static str) -> ConfigurationError {
    ConfigurationError::new(path.display().to_string(), reason)
}

pub(super) fn redirected(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(super) fn read_file(
    path: &Path,
    limit: usize,
    no_links: bool,
    observed_bytes: &mut u64,
) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).map_err(|_| at(path, "cannot inspect configuration file"))?;
    if !metadata.is_file() {
        return Err(at(path, "configuration must be a regular file"));
    }
    *observed_bytes = (*observed_bytes).max(metadata.len());
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | if no_links { libc::O_NOFOLLOW } else { 0 });
    }
    #[cfg(windows)]
    if no_links {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|_| at(path, "cannot open configuration file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| at(path, "cannot inspect opened configuration file"))?;
    if !metadata.is_file() || (no_links && redirected(&metadata)) {
        return Err(at(path, "configuration must be an ordinary regular file"));
    }
    *observed_bytes = (*observed_bytes).max(metadata.len());
    if metadata.len() > limit as u64 {
        return Err(at(path, "file exceeds byte limit"));
    }
    let mut bytes = Vec::new();
    let read = file.take(limit as u64 + 1).read_to_end(&mut bytes);
    *observed_bytes = (*observed_bytes).max(bytes.len() as u64);
    read.map_err(|_| at(path, "cannot read configuration file"))?;
    if bytes.len() > limit {
        return Err(at(path, "file exceeds byte limit"));
    }
    Ok(bytes)
}
