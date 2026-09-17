use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor, VoiceGender,
};
use omnivox_tts::{
    AudioBuffer, SynthesisRequest, SynthesisResult, TtsEngine, TtsError, VoiceInfo, VoiceQuality,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use omnivox_tts::voice_library::{AssetFile, MbrolaProfile, RuntimeLibrary, MBROLA_EN1};
pub const VOICE: &str = MBROLA_EN1;
const DATABASE_SHA256: &str = "edb8eaae6f0e38493d88ed627518632e6ff8a3843bcf08474a1a70aa786fd99f";
const MAX_TEXT: usize = 8192;
const MAX_PHO: usize = 1024 * 1024;
const MAX_PCM: usize = 16 * 1024 * 1024;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    voice_id: String,
    sample_rate: u32,
    frontend: String,
    runtime: String,
    database: String,
    files: BTreeMap<String, String>,
    sources: BTreeMap<String, serde_json::Value>,
    builder_sha256: String,
    frontend_overlay_sha256: String,
}

struct Bundle {
    root: PathBuf,
    manifest: Manifest,
}

fn checked_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    if relative.is_empty() || relative.contains('\\') || relative.contains(':') {
        return Err(io::Error::other("invalid private bundle path"));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::other("non-relative bundle path"));
        };
        path.push(name);
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(io::Error::other("bundle symlinks are not supported"));
        }
    }
    Ok(path)
}

impl Bundle {
    fn load(root: PathBuf) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(root.join("prototype.json"))?
            .take(256 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 256 * 1024 {
            return Err(io::Error::other("prototype manifest exceeds limit"));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        if manifest.schema_version != 1
            || manifest.voice_id != VOICE
            || manifest.sample_rate != 16000
            || manifest.database != "espeak-ng-data/mbrola/en1"
            || manifest.frontend != format!("frontend{}", std::env::consts::EXE_SUFFIX)
            || manifest.runtime != format!("mbrola{}", std::env::consts::EXE_SUFFIX)
            || manifest.files.len() > 1024
            || manifest.builder_sha256.len() != 64
            || manifest.sources.len() != 5
            || manifest.frontend_overlay_sha256
                != hex(&Sha256::digest(include_bytes!(
                    "../../tools/mbrola/frontend-only.c"
                )))
        {
            return Err(io::Error::other("unsupported MBROLA prototype manifest"));
        }
        for required in [
            manifest.frontend.as_str(),
            manifest.runtime.as_str(),
            manifest.database.as_str(),
            "espeak-ng-data/voices/mb/mb-en1",
            "espeak-ng-data/mbrola_ph/en1_phtrans",
            "espeak-ng-data/voices/mb/mb-us1",
            "espeak-ng-data/voices/mb/mb-us2",
            "espeak-ng-data/voices/mb/mb-us3",
            "espeak-ng-data/mbrola_ph/us_phtrans",
            "espeak-ng-data/mbrola_ph/us3_phtrans",
            "espeak-ng-data/phontab",
            "espeak-ng-data/phondata",
            "espeak-ng-data/phonindex",
            "espeak-ng-data/en_dict",
            "espeak-ng-data/intonations",
        ] {
            if !manifest.files.contains_key(required) {
                return Err(io::Error::other("incomplete MBROLA prototype bundle"));
            }
        }
        if manifest.files[&manifest.database] != DATABASE_SHA256 {
            return Err(io::Error::other("unreviewed MBROLA database"));
        }
        let bundle = Self { root, manifest };
        bundle.verify()?;
        Ok(bundle)
    }

    fn verify(&self) -> io::Result<()> {
        // Reject added voice files too: a new alias could change native lookup
        // even if every file named by the manifest still has its original hash.
        let mut remaining = 2048;
        self.verify_data_paths("espeak-ng-data", 0, &mut remaining)?;
        let mut total = 0u64;
        for (relative, expected) in &self.manifest.files {
            let path = checked_path(&self.root, relative)?;
            let metadata = path.metadata()?;
            total = total.saturating_add(metadata.len());
            if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 || total > 96 * 1024 * 1024
            {
                return Err(io::Error::other("private bundle exceeds file limits"));
            }
            let mut file = File::open(&path)?;
            let mut digest = Sha256::new();
            let mut buffer = [0; 32 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if hex(&digest.finalize()) != *expected {
                return Err(io::Error::other(format!(
                    "private bundle hash mismatch: {relative}"
                )));
            }
        }
        Ok(())
    }

    fn verify_data_paths(
        &self,
        relative: &str,
        depth: usize,
        remaining: &mut usize,
    ) -> io::Result<()> {
        if depth > 16 || *remaining == 0 {
            return Err(io::Error::other("prototype data tree exceeds bounds"));
        }
        *remaining -= 1;
        let path = checked_path(&self.root, relative)?;
        if path.is_dir() {
            for entry in path.read_dir()? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| io::Error::other("non-UTF-8 data path"))?;
                self.verify_data_paths(&format!("{relative}/{name}"), depth + 1, remaining)?;
            }
        } else if !self.manifest.files.contains_key(relative) {
            return Err(io::Error::other("unverified file in prototype data tree"));
        }
        Ok(())
    }
}

pub struct MbrolaEngine {
    bundle: Result<Bundle, String>,
    voices: Vec<ManagedVoice>,
    lock: Mutex<()>,
    cancelled: AtomicU64,
    speaking: AtomicBool,
}

struct ManagedVoice {
    profile: MbrolaProfile,
    database: Option<AssetFile>,
    descriptor: VoiceDescriptor,
}

impl MbrolaEngine {
    pub fn new(root: PathBuf, library: Option<&RuntimeLibrary>) -> Self {
        let mut voices = Vec::new();
        let managed = library.and_then(|library| library.document().mbrola.as_ref());
        let mut bundle = Bundle::load(root).map_err(|e| e.to_string());
        if managed.is_none_or(|managed| managed.builtin_en1) {
            voices.push(ManagedVoice {
                profile: MbrolaProfile::for_id(VOICE).unwrap(),
                database: None,
                descriptor: VoiceDescriptor {
                    id: PhysicalVoiceId::new("mbrola", VOICE),
                    display_name: "MBROLA en1 (Roger, British English)".to_owned(),
                    language: Some("en-GB".to_owned()),
                    gender: Some(VoiceGender::Male),
                    quality: VoiceQuality::Compact,
                    availability: Availability::Available,
                },
            });
        }
        if let Some(managed) = managed {
            for voice in &managed.files {
                // Metadata was structurally checked by RuntimeLibrary. Verify
                // every selected asset once at startup; synthesize checks only
                // the selected database, without retaining any native handles.
                if let Err(error) = voice.database.open_verified() {
                    bundle = Err(error.to_string());
                }
                voices.push(ManagedVoice {
                    profile: MbrolaProfile::for_id(&voice.physical_id).expect("validated profile"),
                    database: Some(voice.database.clone()),
                    descriptor: VoiceDescriptor {
                        id: PhysicalVoiceId::new("mbrola", &voice.physical_id),
                        display_name: voice.display_name.clone(),
                        language: voice.language.clone(),
                        gender: None,
                        quality: VoiceQuality::Compact,
                        availability: Availability::Available,
                    },
                });
            }
        }
        Self {
            bundle,
            voices,
            lock: Mutex::new(()),
            cancelled: AtomicU64::new(0),
            speaking: AtomicBool::new(false),
        }
    }
}

struct Speaking<'a>(&'a AtomicBool);
impl Drop for Speaking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn failure(error: impl std::fmt::Display) -> TtsError {
    TtsError::SynthesisFailed(error.to_string())
}

impl TtsEngine for MbrolaEngine {
    fn descriptor(&self) -> EngineDescriptor {
        if let Err(reason) = &self.bundle {
            return EngineDescriptor::unavailable("mbrola", reason);
        }
        EngineDescriptor {
            id: "mbrola".to_owned(),
            display_name: "MBROLA (English prototype)".to_owned(),
            version: Some("274dead162f28 / en1 fe05a0ccef6a".to_owned()),
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: capabilities(),
            espeak_variants: Vec::new(),
            default_voice_id: self
                .voices
                .first()
                .map(|voice| voice.profile.physical_id.to_owned()),
            voices: self
                .voices
                .iter()
                .map(|voice| voice.descriptor.clone())
                .collect(),
        }
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let voice = request.voice_id_for_engine("mbrola")?;
        let selected = self
            .voices
            .iter()
            .find(|selected| selected.profile.physical_id == voice)
            .ok_or_else(|| TtsError::VoiceNotFound(voice.to_owned()))?;
        let bundle = self.bundle.as_ref().map_err(failure)?;
        if request.text.len() > MAX_TEXT || request.text.contains('\0') {
            return Err(TtsError::InvalidParameter(
                "MBROLA prototype accepts at most 8192 UTF-8 bytes without NUL".to_owned(),
            ));
        }
        if ![
            request.settings.rate,
            request.settings.pitch,
            request.settings.volume,
        ]
        .iter()
        .all(|v| v.is_finite())
        {
            return Err(TtsError::InvalidParameter(
                "MBROLA controls must be finite".to_owned(),
            ));
        }
        // Capture before waiting for serialization: a stop must also cancel queued work.
        let generation = self.cancelled.load(Ordering::Acquire);
        let cancelled = || {
            self.cancelled.load(Ordering::Acquire) != generation
                || request
                    .cancellation
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
        };
        let _lock = self.lock.lock().map_err(failure)?;
        if cancelled() {
            return Err(failure("MBROLA request already cancelled"));
        }
        self.speaking.store(true, Ordering::Release);
        let _speaking = Speaking(&self.speaking);
        bundle.verify().map_err(failure)?;
        let database = if let Some(database) = &selected.database {
            database.open_verified().map_err(failure)?;
            PathBuf::from(&database.path)
        } else {
            bundle.root.join(&bundle.manifest.database)
        };
        let actual_voice = Some(PhysicalVoiceId::new("mbrola", voice));
        if request.text.trim().is_empty() {
            return Ok(SynthesisResult::audio(
                "mbrola",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }
        // Provisional native curve. This prototype makes no calibrated-rate claim.
        let rate = omnivox_tts::rate_calibration::interpolate(
            request.settings.rate,
            &[(0.0, 80.0), (0.5, 175.0), (1.0, 350.0), (2.0, 450.0)],
        )
        .round() as u32;
        let pitch = (request.settings.pitch.clamp(0.5, 2.0) * 50.0).round() as u32;
        let mut frontend = Command::new(bundle.root.join(&bundle.manifest.frontend));
        frontend.args([
            "--pho",
            "-q",
            "-v",
            selected.profile.frontend,
            "--stdin",
            "-s",
            &rate.to_string(),
            "-p",
            &pitch.to_string(),
        ]);
        frontend.arg(format!("--path={}", bundle.root.display()));
        // This pinned frontend's bulk stdin reader replaces its final input
        // byte with NUL. Supply that terminator ourselves so it cannot discard
        // the last character (or part of a UTF-8 character) of spoken text.
        let mut input = request.text.as_bytes().to_vec();
        input.push(0);
        let pho = crate::process::capture(
            &mut frontend,
            input,
            MAX_PHO,
            Duration::from_secs(10),
            cancelled,
        )
        .map_err(failure)?;
        if pho.is_empty() || !pho.is_ascii() {
            return Err(failure("frontend produced no valid phoneme instructions"));
        }
        let mut runtime = Command::new(bundle.root.join(&bundle.manifest.runtime));
        let volume = request.settings.volume.clamp(0.0, 1.0);
        // MBROLA rejects -v 0. Preserve utterance timing, then mute its PCM.
        runtime.args(["-v", &if volume == 0.0 { 1.0 } else { volume }.to_string()]);
        runtime.arg(database).args(["-", "-"]);
        let pcm = crate::process::capture(
            &mut runtime,
            pho,
            MAX_PCM,
            Duration::from_secs(20),
            cancelled,
        )
        .map_err(failure)?;
        if cancelled() {
            return Err(failure("MBROLA request cancelled before PCM commitment"));
        }
        if pcm.is_empty() || pcm.len() % 2 != 0 {
            return Err(failure("MBROLA returned invalid PCM"));
        }
        let samples: Vec<_> = pcm
            .chunks_exact(2)
            .map(|pair| {
                if volume == 0.0 {
                    0
                } else {
                    i16::from_le_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        let audio =
            AudioBuffer::try_from_interleaved_i16(&samples, selected.profile.sample_rate, 1)
                .map_err(failure)?;
        let mut result = SynthesisResult::audio("mbrola", actual_voice, audio);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities().acss)
            .omitted;
        Ok(result)
    }

    fn stop(&self) {
        self.cancelled.fetch_add(1, Ordering::AcqRel);
    }
    fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
    }
    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor()
            .voices
            .into_iter()
            .map(|v| VoiceInfo {
                identifier: v.id.voice_id,
                name: v.display_name,
                language: v.language.unwrap_or_default(),
                quality: v.quality,
            })
            .collect()
    }
    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|v| v.identifier == identifier)
    }
}

fn capabilities() -> EngineCapabilities {
    EngineCapabilities {
        acss: AcssCapabilities {
            rate: true,
            average_pitch: true,
            volume: true,
            ..AcssCapabilities::default()
        },
        audio_output: AudioOutputMode::BufferedPcm,
        cancellation: CancellationSupport::SynthesisAndPlayback,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities::default(),
        language_switching: false,
        text_repertoire: TextRepertoire::default(),
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_bundle_never_advertises_a_voice_or_accepts_base_alias() {
        let engine =
            MbrolaEngine::new(PathBuf::from("/nonexistent-omnivox-mbrola-prototype"), None);
        assert!(engine.available_voices().is_empty());
        assert!(!engine.descriptor().availability.is_available());
        assert!(matches!(
            engine.synthesize(&SynthesisRequest::new(
                "text",
                omnivox_tts::TtsSettings {
                    voice: "mb-en1".to_owned(),
                    ..Default::default()
                }
            )),
            Err(TtsError::VoiceNotFound(_))
        ));
    }
}
