//! Local engine drafts and atomic saves. No engine construction or activation.
use super::*;
use punctuation_editor as storage;
use std::fs;

#[derive(Debug, Serialize)]
pub struct Engine {
    pub engine_id: String,
    pub in_process: bool,
    pub enabled: bool,
    pub program: Option<String>,
    pub environment_override: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub path: String,
    pub sha256: String,
    pub settings: Value,
    pub engines: Vec<Engine>,
    pub diagnostics: Vec<String>,
}

fn failure(reason: &'static str) -> ConfigurationError {
    ConfigurationError::new("engine settings", reason)
}

pub fn inspect(root: &ConfigurationRoot) -> Result<Review> {
    let path = storage::path(root)?;
    let bytes = storage::read(&path)?;
    let document = storage::document(bytes.as_deref())?;
    let loaded = root.load()?;
    let environment = LaunchEnvironment::capture();
    let mut engines = shipped::ENGINES
        .iter()
        .map(|engine| Engine {
            engine_id: engine.id.into(),
            in_process: engine.in_process,
            enabled: true,
            program: None,
            environment_override: engine.helper_environment.and_then(|name| {
                environment
                    .get(name)
                    .filter(|value| !value.is_empty())
                    .map(|_| name.to_owned())
            }),
        })
        .collect::<Vec<_>>();
    engines.extend(loaded.external.values().map(|entry| Engine {
        engine_id: entry.manifest.engine_id.clone(),
        in_process: false,
        enabled: entry.manifest.enabled,
        program: Some(entry.manifest.program.clone()),
        environment_override: None,
    }));
    Ok(Review {
        path: path.to_string_lossy().into_owned(),
        sha256: storage::revision(&path, bytes.as_deref()),
        settings: serde_json::json!({
            "routing": document.get("routing").cloned().unwrap_or_else(|| serde_json::json!({})),
            "engine_overrides": document.get("engine_overrides").cloned().unwrap_or_else(|| serde_json::json!({}))
        }),
        engines,
        diagnostics: loaded.diagnostics.iter().map(ToString::to_string).collect(),
    })
}

pub fn save(root: &ConfigurationRoot, expected: &str, settings_json: &[u8]) -> Result<Review> {
    let settings = json::parse_snapshot(settings_json, MAX_CONFIG_BYTES)?;
    let object = settings
        .as_object()
        .ok_or_else(|| failure("settings must be an object"))?;
    if object.len() != 2
        || !object.contains_key("routing")
        || !object.contains_key("engine_overrides")
    {
        return Err(failure("provide routing and engine_overrides only"));
    }
    let path = storage::path(root)?;
    let original = storage::read(&path)?;
    if storage::revision(&path, original.as_deref()) != expected {
        return Err(failure(
            "file or target changed; refresh and review your edits",
        ));
    }
    let mut document = storage::document(original.as_deref())?;
    for (name, value) in object {
        document[name] = value.clone();
    }
    let mut bytes = serde_json::to_vec_pretty(&document)
        .map_err(|_| failure("cannot encode engine settings"))?;
    bytes.push(b'\n');
    let configuration = Configuration::parse(&bytes, Platform::native())?;
    let loaded = root.load()?;
    configuration.validate_overrides(&loaded.external.keys().cloned().collect())?;
    storage::publish(&path, original, &bytes)?;
    inspect(root)
}

/// Install one reviewed external registration, disabled until a later Apply.
pub fn add_helper(
    root: &ConfigurationRoot,
    expected: &str,
    manifest_json: &[u8],
) -> Result<Review> {
    let manifest = HelperManifest::parse(manifest_json, Platform::native())?;
    if manifest.enabled || shipped::reserved(&manifest.engine_id) {
        return Err(failure(
            "new helpers must be disabled and use a non-reserved ID",
        ));
    }
    if !std::path::Path::new(&manifest.program).is_file() {
        return Err(failure(
            "choose an installed helper program on this speech host",
        ));
    }
    let review = inspect(root)?;
    if review.sha256 != expected {
        return Err(failure(
            "file or target changed; refresh and review your edits",
        ));
    }
    if review
        .engines
        .iter()
        .any(|engine| engine.engine_id == manifest.engine_id)
    {
        return Err(failure("engine is already registered"));
    }
    let directory = storage::path(root)?.parent().unwrap().join("helpers.d");
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        if !metadata.is_dir() || files::redirected(&metadata) {
            return Err(failure("helpers.d must be an ordinary directory"));
        }
    }
    let path = directory.join(format!("{}.json", manifest.engine_id));
    if fs::symlink_metadata(&path).is_ok() {
        return Err(failure("registration file already exists"));
    }
    storage::publish_checked(&path, None, manifest_json, || {
        let mut count = 0;
        let mut total = manifest_json.len() as u64;
        for entry in fs::read_dir(&directory).map_err(|_| failure("cannot read helpers.d"))? {
            let entry = entry.map_err(|_| failure("cannot read helper registration"))?;
            if entry
                .path()
                .extension()
                .is_some_and(|suffix| suffix == "json")
            {
                count += 1;
                total += fs::symlink_metadata(entry.path())
                    .map_err(|_| failure("cannot inspect helper registration"))?
                    .len();
            }
        }
        if count >= MAX_MANIFESTS || total > MAX_MANIFEST_TOTAL_BYTES as u64 {
            return Err(failure("helper registration limit reached"));
        }
        Ok(())
    })?;
    inspect(root)
}

#[cfg(test)]
mod tests;
