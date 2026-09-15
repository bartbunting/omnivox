//! Real native failure/recovery coverage, explicitly run with reviewed assets.
use std::ffi::CString;
use std::path::{Path, PathBuf};

use omnivox_piper_sys::*;

struct Synth(*mut piper_synthesizer);

impl Drop for Synth {
    fn drop(&mut self) {
        unsafe { piper_free(self.0) };
    }
}

fn create(model: &Path, config: &Path, data: &Path) -> Synth {
    let model = CString::new(model.to_str().unwrap()).unwrap();
    let config = CString::new(config.to_str().unwrap()).unwrap();
    let data = CString::new(data.to_str().unwrap()).unwrap();
    let options = piper_create_options {
        struct_size: std::mem::size_of::<piper_create_options>(),
        model_path: model.as_ptr(),
        config_path: config.as_ptr(),
        espeak_data_path: data.as_ptr(),
    };
    Synth(unsafe { piper_create_with_options(&options) })
}

fn speak(synth: &Synth) {
    let text = CString::new("Piper can recover after a failed model load.").unwrap();
    let options = unsafe { piper_default_synthesize_options(synth.0) };
    assert_eq!(
        unsafe { piper_synthesize_start(synth.0, text.as_ptr(), &options) },
        PIPER_OK as i32
    );
    let mut samples = 0;
    for _ in 0..100 {
        let mut chunk: piper_audio_chunk = unsafe { std::mem::zeroed() };
        let status = unsafe { piper_synthesize_next(synth.0, &mut chunk) };
        assert!(status == PIPER_OK as i32 || status == PIPER_DONE as i32);
        samples += chunk.num_samples;
        if status == PIPER_DONE as i32 || chunk.is_last {
            assert!(samples > 0);
            return;
        }
    }
    panic!("native synthesis did not finish within 100 chunks");
}

#[test]
fn failed_construction_and_inference_leave_one_helper_reusable() {
    let model =
        PathBuf::from(std::env::var_os("OMNIVOX_PIPER_TEST_MODEL").expect("set test model"));
    let config = PathBuf::from(format!("{}.json", model.display()));
    let data = std::env::var_os("OMNIVOX_PIPER_ESPEAK_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(PIPER_ESPEAK_DATA_DIR));
    let root = std::env::temp_dir().join(format!(
        "omnivox-native-piper-lifecycle-{}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    let bad_model = root.join("bad.onnx");
    let bad_config = root.join("bad.json");
    std::fs::write(&bad_model, b"not an ONNX model").unwrap();
    std::fs::write(&bad_config, b"not JSON").unwrap();
    for _ in 0..3 {
        assert!(create(&model, &bad_config, &data).0.is_null());
        assert!(create(&bad_model, &config, &data).0.is_null());
        let good = create(&model, &config, &data);
        assert!(!good.0.is_null());
        // A second attempted construction must not free the first phonemizer.
        assert!(create(&model, &config, &data).0.is_null());
        speak(&good);
    }
    // Deliberately mismatch the single-speaker graph and config to provoke
    // an ONNX inference exception, which must return a C error, not abort.
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("\"num_speakers\": 1"),
        "fixture must be single-speaker"
    );
    std::fs::write(
        &bad_config,
        text.replace("\"num_speakers\": 1", "\"num_speakers\": 2"),
    )
    .unwrap();
    {
        let incompatible = create(&model, &bad_config, &data);
        assert!(!incompatible.0.is_null());
        let text = CString::new("Test incompatible graph.").unwrap();
        let options = unsafe { piper_default_synthesize_options(incompatible.0) };
        assert_eq!(
            unsafe { piper_synthesize_start(incompatible.0, text.as_ptr(), &options) },
            PIPER_OK as i32
        );
        let mut chunk = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { piper_synthesize_next(incompatible.0, &mut chunk) },
            PIPER_ERR_GENERIC
        );
    }
    let good = create(&model, &config, &data);
    assert!(!good.0.is_null());
    speak(&good);
    drop(good);
    std::fs::remove_dir_all(root).unwrap();
}
