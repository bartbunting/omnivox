//! Helper-local model ownership. The manager verifies asset hashes before use;
//! this layer validates selected config/speaker metadata and owns native lifetime.
use super::*;
use std::fs::File;
use std::io::Read;

pub(super) struct ModelSpec {
    pub model: PathBuf,
    pub config: PathBuf,
    pub speakers: Vec<u32>,
    pub expected_bytes: Option<(u64, u64)>,
}

pub(super) struct VoiceBinding {
    pub info: VoiceInfo,
    pub language: Option<String>,
    pub model_index: usize,
    pub speaker_index: u32,
}

pub(super) struct NativeModel(pub *mut omnivox_piper_sys::piper_synthesizer);

// SAFETY: NativeModel has one owner, inside the engine's state mutex. Native
// calls never outlive that lock. The native overlay additionally prevents two
// models from sharing the process-global phonemizer at the same time.
unsafe impl Send for NativeModel {}

impl Drop for NativeModel {
    fn drop(&mut self) {
        unsafe { omnivox_piper_sys::piper_free(self.0) };
    }
}

pub(super) struct ResidentModel {
    pub model_index: usize,
    pub native: NativeModel,
}

impl ModelSpec {
    pub fn load(&self) -> Result<NativeModel, TtsError> {
        let unavailable =
            |reason: &str| TtsError::VoiceNotFound(format!("Piper model unavailable: {reason}"));
        let model_metadata = std::fs::metadata(&self.model)
            .map_err(|_| unavailable("model file is missing or unreadable"))?;
        if !model_metadata.is_file() {
            return Err(unavailable("model path is not a file"));
        }
        let config =
            File::open(&self.config).map_err(|_| unavailable("configuration is unreadable"))?;
        let config_metadata = config
            .metadata()
            .map_err(|_| unavailable("configuration metadata is unreadable"))?;
        if !config_metadata.is_file() {
            return Err(unavailable("configuration path is not a file"));
        }
        if self.expected_bytes.is_some_and(|(model, config)| {
            model != model_metadata.len() || config != config_metadata.len()
        }) {
            return Err(unavailable(
                "asset size changed since the library was prepared",
            ));
        }
        const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
        let mut bytes = Vec::new();
        config
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable("could not read configuration"))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(unavailable("configuration exceeds 1 MiB"));
        }
        serde_json::from_slice::<crate::control::DuplicateFreeJson>(&bytes)
            .map_err(|_| unavailable("configuration is invalid or contains duplicate keys"))?;
        let config: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| unavailable("configuration is not valid JSON"))?;
        let count = config
            .get("num_speakers")
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0 && *n <= i32::MAX as u64)
            .ok_or_else(|| unavailable("configuration has no valid speaker count"))?;
        if self.speakers.iter().any(|index| u64::from(*index) >= count) {
            return Err(unavailable(
                "enabled speaker is outside the native speaker range",
            ));
        }
        let data = match config
            .get("phoneme_type")
            .and_then(serde_json::Value::as_str)
        {
            None | Some("espeak") => {
                Some(find_espeak_data().ok_or_else(|| unavailable("phonemizer data is missing"))?)
            }
            Some("text") => None,
            Some(_) => return Err(unavailable("unsupported phoneme type")),
        };
        let model = CString::new(self.model.to_string_lossy().as_ref())
            .map_err(|_| unavailable("invalid model path"))?;
        let config = CString::new(self.config.to_string_lossy().as_ref())
            .map_err(|_| unavailable("invalid configuration path"))?;
        let data = data
            .map(|path| CString::new(path.to_string_lossy().as_ref()))
            .transpose()
            .map_err(|_| unavailable("invalid phonemizer data path"))?;
        let options = omnivox_piper_sys::piper_create_options {
            struct_size: std::mem::size_of::<omnivox_piper_sys::piper_create_options>(),
            model_path: model.as_ptr(),
            config_path: config.as_ptr(),
            espeak_data_path: data.as_ref().map_or(std::ptr::null(), |data| data.as_ptr()),
        };
        let ptr = unsafe { omnivox_piper_sys::piper_create_with_options(&options) };
        if ptr.is_null() {
            return Err(unavailable("native loading failed; see helper diagnostics"));
        }
        Ok(NativeModel(ptr))
    }
}
