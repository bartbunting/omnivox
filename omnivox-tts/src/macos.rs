//! macOS TTS Engine using AVSpeechSynthesizer
//!
//! Uses an Objective-C bridge (macos_bridge.m) for buffer capture via
//! AVSpeechSynthesizer.write(_:toBufferCallback:). The bridge is compiled
//! by build.rs and linked in statically.

use crate::{AudioBuffer, TtsEngine, TtsError, TtsSettings, VoiceInfo, VoiceQuality};
use tracing::{debug, info, warn};

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
    fn omnivox_list_voices() -> VoiceList;
    fn omnivox_free_voice_list(list: VoiceList);
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
    fn synthesize(&self, text: &str, settings: &TtsSettings) -> Result<AudioBuffer, TtsError> {
        debug!(
            "Synthesizing: {} (rate: {}, pitch: {}, volume: {})",
            text, settings.rate, settings.pitch, settings.volume
        );

        if text.is_empty() {
            return Ok(AudioBuffer::empty());
        }

        let c_text = std::ffi::CString::new(text)
            .map_err(|_| TtsError::SynthesisFailed("Invalid text".to_string()))?;

        let (lang, name) = Self::parse_voice_id(&settings.voice);

        let c_lang = lang
            .as_ref()
            .and_then(|l| std::ffi::CString::new(l.as_str()).ok());
        let c_name = name
            .as_ref()
            .and_then(|n| std::ffi::CString::new(n.as_str()).ok());

        let lang_ptr = c_lang
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr());
        let name_ptr = c_name
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr());

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
            return Ok(AudioBuffer::empty());
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

        let buffer = AudioBuffer::new(samples, result.sample_rate, result.channels);
        Ok(buffer.to_standard_format())
    }

    fn stop(&self) {
        debug!("Stop requested (bridge engine creates new synthesizer per call)");
    }

    fn is_speaking(&self) -> bool {
        false
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
    fn synthesize(&self, _text: &str, _settings: &TtsSettings) -> Result<AudioBuffer, TtsError> {
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
