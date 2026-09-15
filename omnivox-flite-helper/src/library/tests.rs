use super::*;
use omnivox_tts::voice_library::{HostPlatform, RuntimeLibrary};
use omnivox_tts::TtsSettings;

// The already linked upstream exporter creates disposable native test data.
// No new voice download, FFI production API or distributed model is needed.
unsafe extern "C" {
    fn flite_voice_dump(voice: *mut FliteVoice, filename: *const std::ffi::c_char) -> c_int;
    static mut cmu_us_slt_cg: *mut FliteVoice;
}

#[test]
fn export_bundled_fixture() {
    let Some(path) = env::var_os("OMNIVOX_FLITE_TEST_EXPORT") else {
        return;
    };
    assert!(unsafe { cmu_us_slt_cg }.is_null());
    let fixture = Fixture::new();
    let empty = FliteTtsEngine::from_library(&fixture.library(false, &[])).unwrap();
    // Observe upstream's actual registration slot in this fresh process.
    assert!(unsafe { cmu_us_slt_cg }.is_null());
    drop(empty);
    // Run in a fresh owned process. The shared built-in voice in the parent
    // may have numeric synthesis settings that upstream's exporter cannot read.
    let _global = lock_flite_global().unwrap();
    assert_eq!(unsafe { omnivox_flite_sys::omnivox_flite_initialize() }, 0);
    let voice = unsafe { omnivox_flite_sys::omnivox_flite_register_slt() };
    assert!(!voice.is_null());
    let path = CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { flite_voice_dump(voice, path.as_ptr()) }, 1);
}

#[derive(Default)]
struct PcmSink {
    voice: Option<PhysicalVoiceId>,
    frames: u64,
}

impl SynthesisStreamSink for PcmSink {
    fn start(&mut self, start: SynthesisStreamStart) -> Result<(), TtsError> {
        self.voice = start.actual_voice;
        Ok(())
    }
    fn audio(&mut self, audio: AudioBuffer) -> Result<(), TtsError> {
        self.frames += audio.frame_count() as u64;
        Ok(())
    }
    fn markers(&mut self, _: Vec<SynthesisMarker>, _: Vec<ResolvedAnchor>) -> Result<(), TtsError> {
        Ok(())
    }
}

fn export_fixture(path: &Path) {
    let mut child = std::process::Command::new(env::current_exe().unwrap())
        .args(["--exact", "library::tests::export_bundled_fixture"])
        .env("OMNIVOX_FLITE_TEST_EXPORT", path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "native fixture exporter failed: {status}");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native fixture exporter exceeded its deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = env::temp_dir().join(format!("omnivox-flite-library-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn library(&self, builtin: bool, entries: &[(&str, &str, u64)]) -> RuntimeLibrary {
        use sha2::{Digest, Sha256};
        let files = entries.iter().map(|(id, name, bytes)| {
            let hash: String = Sha256::digest(std::fs::read(self.0.join(name)).unwrap_or_default())
                .iter().map(|byte| format!("{byte:02x}")).collect();
            let path = self.0.join(name).to_str().unwrap().replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"physical_id":"{id}","file":{{"path":"{path}","bytes":{bytes},"sha256":"{hash}"}},"display_name":"Local test voice","language":null}}"#)
        }).collect::<Vec<_>>().join(",");
        let source = format!(
            r#"{{"schema_version":1,"target_id":"11111111-1111-4111-8111-111111111111","profile_id":"22222222-2222-4222-8222-222222222222","generation_id":"33333333-3333-4333-8333-333333333333","disabled_physical_ids":[],"piper":null,"flite":{{"builtin_slt":{builtin},"files":[{files}]}}}}"#
        );
        RuntimeLibrary::parse(
            source.as_bytes(),
            if cfg!(windows) {
                HostPlatform::Windows
            } else {
                HostPlatform::Posix
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn managed_native_files_enforce_identity_enablement_and_complete_loading() {
    let fixture = Fixture::new();
    let empty = FliteTtsEngine::from_library(&fixture.library(false, &[])).unwrap();
    assert!(empty.runtime().unwrap().voices.is_empty());
    assert!(empty.available_voices().is_empty());
    assert!(empty.descriptor().default_voice_id.is_none());
    assert!(!empty.descriptor().can_synthesize());
    let request = |id: &str| {
        SynthesisRequest::new(
            "Voice selection works.",
            TtsSettings {
                voice: id.to_owned(),
                ..TtsSettings::default()
            },
        )
    };
    assert!(matches!(
        empty.synthesize(&request(BUILT_IN_VOICE_ID)),
        Err(TtsError::VoiceNotFound(_))
    ));
    drop(empty);

    let builtin = FliteTtsEngine::from_library(&fixture.library(true, &[])).unwrap();
    assert_eq!(builtin.available_voices().len(), 1);
    assert!(!builtin
        .synthesize(&request(BUILT_IN_VOICE_ID))
        .unwrap()
        .audio
        .is_empty());
    let path = fixture.0.join("unrelated-filename.flitevox");
    export_fixture(&path);
    drop(builtin);
    let size = std::fs::metadata(&path).unwrap().len();
    let id = {
        let _global = lock_flite_global().unwrap();
        let voice = load_external_voice(&path, None).unwrap();
        let id = voice.id.clone();
        unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(voice.pointer) };
        id
    };
    let entries = [(id.as_str(), "unrelated-filename.flitevox", size)];
    let library = fixture.library(false, &entries);
    let external = FliteTtsEngine::from_library(&library).unwrap();
    let descriptor = external.descriptor();
    assert_eq!(descriptor.voices.len(), 1);
    assert_eq!(descriptor.default_voice_id.as_deref(), Some(id.as_str()));
    assert_eq!(descriptor.voices[0].display_name, "Local test voice");
    assert_eq!(descriptor.voices[0].language, None);
    assert!(matches!(
        external.synthesize(&request(BUILT_IN_VOICE_ID)),
        Err(TtsError::VoiceNotFound(_))
    ));
    let result = external.synthesize(&request(&id)).unwrap();
    assert_eq!(result.actual_voice.unwrap().voice_id, id);
    assert!(!result.audio.is_empty());
    let mut sink = PcmSink::default();
    let completion = external
        .synthesize_stream(&request(&id), &mut sink)
        .unwrap();
    assert_eq!(sink.voice.unwrap().voice_id, id);
    assert!(sink.frames > 0);
    assert_eq!(completion.frame_count, sink.frames);
    assert!(matches!(
        external.synthesize_stream(&request(BUILT_IN_VOICE_ID), &mut PcmSink::default()),
        Err(TtsError::VoiceNotFound(_))
    ));
    // Managed mode requires the stable physical ID, never a native-name alias.
    let alias = id.strip_prefix("flitevox:").unwrap();
    assert!(matches!(
        external.synthesize(&request(alias)),
        Err(TtsError::VoiceNotFound(_))
    ));
    drop(external);

    for bad in [
        fixture.library(
            false,
            &[("flitevox:wrong-name", "unrelated-filename.flitevox", size)],
        ),
        fixture.library(
            false,
            &[(id.as_str(), "unrelated-filename.flitevox", size + 1)],
        ),
        fixture.library(false, &[(id.as_str(), "missing.flitevox", size)]),
    ] {
        assert!(matches!(
            FliteTtsEngine::from_library(&bad),
            Err(TtsError::VoiceNotFound(_))
        ));
    }
    // A later failure must clean up earlier native loads and reject the entire
    // selection. Sorted IDs place this mismatched second file after SLT.
    let partial = fixture.library(
        false,
        &[
            (id.as_str(), "unrelated-filename.flitevox", size),
            (
                "flitevox:zz-wrong-name",
                "unrelated-filename.flitevox",
                size,
            ),
        ],
    );
    assert!(matches!(
        FliteTtsEngine::from_library(&partial),
        Err(TtsError::VoiceNotFound(_))
    ));
    let recovered = FliteTtsEngine::from_library(&library).unwrap();
    assert!(!recovered
        .synthesize(&request(&id))
        .unwrap()
        .audio
        .is_empty());
    drop(recovered);

    let mut warnings = Vec::new();
    let legacy = FliteTtsEngine::new(
        vec![path, fixture.0.join("missing.flitevox")],
        &mut warnings,
    )
    .unwrap();
    assert_eq!(legacy.available_voices().len(), 2);
    assert_eq!(
        legacy.descriptor().default_voice_id.as_deref(),
        Some(BUILT_IN_VOICE_ID)
    );
    assert_eq!(warnings.len(), 1);
    drop(legacy);
    // A same-size edit after generation creation must fail before native load.
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut changed = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.0.join("unrelated-filename.flitevox"))
        .unwrap();
    let mut byte = [0];
    changed.read_exact(&mut byte).unwrap();
    changed.seek(SeekFrom::Start(0)).unwrap();
    changed.write_all(&[byte[0] ^ 1]).unwrap();
    drop(changed);
    assert!(
        matches!(FliteTtsEngine::from_library(&library), Err(TtsError::VoiceNotFound(reason)) if reason.contains("SHA-256"))
    );
    // In particular on Windows, outstanding native file handles must not
    // prevent removal after successful and partially failed selections.
    std::fs::remove_file(fixture.0.join("unrelated-filename.flitevox")).unwrap();
}
