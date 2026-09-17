use super::*;
use omnivox_tts::voice_library::RuntimeLibrary;

impl RhVoiceTtsEngine {
    pub fn from_library(library: &RuntimeLibrary) -> Result<Self, TtsError> {
        let load = || -> Result<Self, Box<dyn std::error::Error>> {
            let managed = library
                .document()
                .rhvoice
                .as_ref()
                .ok_or("missing RHVoice load set")?;
            library.verify_assets(Default::default())?;
            let mut config = if managed.inherit_external {
                RuntimeConfig::from_environment()?
            } else {
                RuntimeConfig {
                    library: absolute_file_from_env(ENV_LIBRARY)?,
                    ..Default::default()
                }
            };
            if !managed.inherit_external {
                // RHVoice joins config_path with "dicts" and rejects an empty
                // path. This verified directory contains only the explicit
                // language/voice resources: no implicit languages/, voices/,
                // dictionaries or configuration files can be discovered here.
                let voice = managed
                    .voices
                    .first()
                    .ok_or("empty RHVoice validation set")?;
                let isolated = PathBuf::from(voice.root()?).join("rhvoice");
                config.data = Some(isolated.clone());
                config.config = Some(isolated);
                config.resources.clear();
            } else {
                // A managed download must not silently shadow a separately
                // installed voice with the same native identity.
                if !managed.voices.is_empty() {
                    if let Ok(external) = Self::load(config.clone()) {
                        if external.descriptor().voices.iter().any(|native| {
                            managed
                                .voices
                                .iter()
                                .any(|voice| voice.physical_id == native.id.voice_id)
                        }) {
                            return Err(
                                "managed RHVoice voice conflicts with a separately installed voice"
                                    .into(),
                            );
                        }
                    }
                }
            }
            let mut language_files = None;
            for voice in &managed.voices {
                let package_root = voice.root()?;
                let language_prefix = format!("{package_root}/rhvoice/language/");
                let root = PathBuf::from(package_root);
                let mut language: Vec<_> = voice
                    .files
                    .iter()
                    .filter_map(|file| {
                        let path = file.path.replace('\\', "/");
                        path.strip_prefix(&language_prefix)
                            .map(|name| (name.to_owned(), file.bytes, file.sha256.clone()))
                    })
                    .collect();
                language.sort();
                if let Some(previous) = &language_files {
                    if previous != &language {
                        return Err(
                            "enabled RHVoice packages require different English data".into()
                        );
                    }
                } else {
                    language_files = Some(language);
                    config.resources.push(root.join("rhvoice/language"));
                }
                config.resources.push(root.join("rhvoice/voice"));
            }
            let engine = Self::load(config)?;
            let descriptor = engine.descriptor();
            for voice in &managed.voices {
                if !descriptor
                    .voices
                    .iter()
                    .any(|v| v.id.voice_id == voice.physical_id)
                {
                    return Err(format!(
                        "RHVoice did not load {} from its managed resources",
                        voice.physical_id
                    )
                    .into());
                }
            }
            if !managed.inherit_external && descriptor.voices.len() != managed.voices.len() {
                return Err("RHVoice returned unexpected validation voices".into());
            }
            Ok(engine)
        };
        load().map_err(|error| TtsError::InvalidParameter(format!("managed RHVoice: {error}")))
    }
}
