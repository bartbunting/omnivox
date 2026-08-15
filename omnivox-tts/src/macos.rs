//! macOS TTS Engine using AVSpeechSynthesizer
//!
//! Uses an Objective-C bridge (macos_bridge.m) for buffer capture via
//! AVSpeechSynthesizer.write(_:toBufferCallback:). The bridge is compiled
//! by build.rs and linked in statically.

use crate::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities,
};
#[cfg(target_os = "macos")]
use crate::contracts::{PhysicalVoiceId, VoiceDescriptor};
#[cfg(target_os = "macos")]
use crate::{AudioBuffer, VoiceQuality};
use crate::{SynthesisRequest, SynthesisResult, TtsEngine, TtsError, VoiceInfo};
#[cfg(target_os = "macos")]
use tracing::{debug, info};

fn macos_capabilities() -> EngineCapabilities {
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
        language_switching: true,
        text_repertoire: crate::contracts::TextRepertoire::Unicode,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct SynthResult {
    samples: *mut f32,
    sample_count: u32,
    sample_rate: u32,
    channels: u16,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct VoiceEntry {
    identifier: *mut std::ffi::c_char,
    name: *mut std::ffi::c_char,
    language: *mut std::ffi::c_char,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct VoiceList {
    entries: *mut VoiceEntry,
    count: u32,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn omnivox_synthesize(
        text: *const std::ffi::c_char,
        voice_lang: *const std::ffi::c_char,
        voice_name: *const std::ffi::c_char,
        rate: f32,
        pitch: f32,
        volume: f32,
    ) -> SynthResult;

    fn omnivox_free_samples(samples: *mut f32);
    fn omnivox_stop();
    fn omnivox_is_speaking() -> bool;
    fn omnivox_list_voices() -> VoiceList;
    fn omnivox_free_voice_list(list: VoiceList);
    fn omnivox_run_main_runloop();
    fn omnivox_stop_main_runloop();
}

/// Block the calling thread running the main NSRunLoop.
///
/// AVSpeechSynthesizer's `writeUtterance:toBufferCallback:` internally
/// dispatches work via the main GCD queue; if the main thread is blocked on
/// raw I/O instead of running a RunLoop, synthesis deadlocks. Call this from
/// `main()` after spawning the reader/server on a background thread.
///
/// Returns when `stop_main_runloop()` is called from another thread.
#[cfg(target_os = "macos")]
pub fn run_main_runloop() {
    unsafe { omnivox_run_main_runloop() }
}

/// Unblock a thread that is in `run_main_runloop()`.
#[cfg(target_os = "macos")]
pub fn stop_main_runloop() {
    unsafe { omnivox_stop_main_runloop() }
}

/// macOS TTS engine using AVSpeechSynthesizer via ObjC bridge
#[cfg(target_os = "macos")]
pub struct MacOsTtsEngine;

unsafe impl Send for MacOsTtsEngine {}
unsafe impl Sync for MacOsTtsEngine {}

#[cfg(target_os = "macos")]
impl MacOsTtsEngine {
    pub fn new() -> Result<Self, TtsError> {
        info!("Initializing macOS TTS engine (ObjC bridge)");
        Ok(Self)
    }

    fn parse_voice_id(voice_id: &str) -> (Option<String>, Option<String>) {
        if let Some((lang, name)) = voice_id.split_once(':') {
            (Some(lang.to_string()), Some(name.to_string()))
        } else {
            (Some(voice_id.to_string()), None)
        }
    }
}

#[cfg(target_os = "macos")]
impl Default for MacOsTtsEngine {
    fn default() -> Self {
        Self::new().expect("Failed to create macOS TTS engine")
    }
}

#[cfg(target_os = "macos")]
impl TtsEngine for MacOsTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        let voices = self
            .available_voices()
            .into_iter()
            .map(|voice| VoiceDescriptor::from_voice_info("macos", voice))
            .collect();

        EngineDescriptor {
            id: "macos".to_owned(),
            display_name: "macOS AVSpeechSynthesizer".to_owned(),
            version: None,
            availability: Availability::Available,
            health: EngineHealth::Healthy,
            capabilities: macos_capabilities(),
            voices,
            default_voice_id: None,
        }
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let text = request.text.as_str();
        let settings = &request.settings;
        debug!(
            "Synthesizing: {} (rate: {}, pitch: {}, volume: {})",
            text, settings.rate, settings.pitch, settings.volume
        );

        if text.is_empty() {
            return Ok(SynthesisResult::audio("macos", None, AudioBuffer::empty()));
        }

        let voice_id = request.voice_id_for_engine("macos")?;
        let selected_voice = self
            .available_voices()
            .into_iter()
            .find(|voice| voice.identifier == voice_id);
        let actual_voice = selected_voice
            .as_ref()
            .map(|voice| PhysicalVoiceId::new("macos", voice.identifier.clone()));

        let c_text = std::ffi::CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Invalid text".to_string()))?;

        let (lang, name) = selected_voice.map_or_else(
            || Self::parse_voice_id(voice_id),
            |voice| (Some(voice.language), Some(voice.name)),
        );

        let c_lang = lang
            .as_ref()
            .and_then(|l| std::ffi::CString::new(l.as_str()).ok());
        let c_name = name
            .as_ref()
            .and_then(|n| std::ffi::CString::new(n.as_str()).ok());

        let lang_ptr = c_lang.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        let name_ptr = c_name.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());

        let result = unsafe {
            omnivox_synthesize(
                c_text.as_ptr(),
                lang_ptr,
                name_ptr,
                settings.rate,
                settings.pitch,
                settings.volume,
            )
        };

        if result.samples.is_null() || result.sample_count == 0 {
            debug!("Synthesis produced no audio data");
            return Ok(SynthesisResult::audio(
                "macos",
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        debug!(
            "Collected {} samples at {}Hz, {} channels",
            result.sample_count, result.sample_rate, result.channels
        );

        // Copy samples from C allocation into Rust Vec
        let samples = unsafe {
            let slice = std::slice::from_raw_parts(result.samples, result.sample_count as usize);
            let vec = slice.to_vec();
            omnivox_free_samples(result.samples);
            vec
        };

        let buffer =
            AudioBuffer::try_from_interleaved_f32(samples, result.sample_rate, result.channels)
                .map_err(|error| {
                    TtsError::SynthesisFailed(format!("could not canonicalize macOS PCM: {error}"))
                })?;
        Ok(SynthesisResult::audio("macos", actual_voice, buffer))
    }

    fn stop(&self) {
        debug!("Stopping speech");
        unsafe { omnivox_stop() };
    }

    fn is_speaking(&self) -> bool {
        unsafe { omnivox_is_speaking() }
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        let list = unsafe { omnivox_list_voices() };
        let mut voices = Vec::with_capacity(list.count as usize);

        if !list.entries.is_null() {
            for i in 0..list.count as usize {
                let entry = unsafe { &*list.entries.add(i) };

                let identifier = unsafe {
                    std::ffi::CStr::from_ptr(entry.identifier)
                        .to_string_lossy()
                        .to_string()
                };
                let name = unsafe {
                    std::ffi::CStr::from_ptr(entry.name)
                        .to_string_lossy()
                        .to_string()
                };
                let language = unsafe {
                    std::ffi::CStr::from_ptr(entry.language)
                        .to_string_lossy()
                        .to_string()
                };

                let quality = if identifier.contains("premium") || name.contains("Premium") {
                    VoiceQuality::Premium
                } else if identifier.contains("enhanced") {
                    VoiceQuality::Enhanced
                } else {
                    VoiceQuality::Compact
                };

                voices.push(VoiceInfo {
                    identifier,
                    name,
                    language,
                    quality,
                });
            }

            unsafe { omnivox_free_voice_list(list) };
        }

        debug!("Found {} voices", voices.len());
        voices
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|v| v.identifier == identifier || v.language == identifier)
    }
}

// Stub implementation for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct MacOsTtsEngine;

#[cfg(not(target_os = "macos"))]
impl MacOsTtsEngine {
    pub fn new() -> Result<Self, TtsError> {
        Err(TtsError::NotAvailable)
    }
}

#[cfg(not(target_os = "macos"))]
impl TtsEngine for MacOsTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        EngineDescriptor {
            id: "macos".to_owned(),
            display_name: "macOS AVSpeechSynthesizer".to_owned(),
            version: None,
            availability: Availability::Unavailable {
                reason: "AVSpeechSynthesizer is only available on macOS".to_owned(),
            },
            health: EngineHealth::Failed {
                reason: "unsupported platform".to_owned(),
            },
            capabilities: macos_capabilities(),
            voices: Vec::new(),
            default_voice_id: None,
        }
    }

    fn synthesize(&self, _request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        Err(TtsError::NotAvailable)
    }

    fn stop(&self) {}

    fn is_speaking(&self) -> bool {
        false
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        vec![]
    }

    fn voice_info(&self, _identifier: &str) -> Option<VoiceInfo> {
        None
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod stub_tests {
    use super::MacOsTtsEngine;
    use crate::TtsEngine;

    #[test]
    fn macos_stub_reports_unavailable() {
        let descriptor = MacOsTtsEngine.descriptor();

        assert_eq!(descriptor.id, "macos");
        assert!(!descriptor.can_synthesize());
        assert!(descriptor.voices.is_empty());
    }
}
