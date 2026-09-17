//! macOS TTS Engine using AVSpeechSynthesizer
//!
//! Uses an Objective-C bridge (macos_bridge.m) for buffer capture via
//! AVSpeechSynthesizer.write(_:toBufferCallback:). The bridge is compiled
//! by build.rs and linked in statically.

#[cfg(any(target_os = "macos", test))]
use crate::contracts::VoiceDescriptor;
use crate::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities,
};
#[cfg(target_os = "macos")]
use crate::VoiceQuality;
use crate::{SynthesisRequest, SynthesisResult, TtsEngine, TtsError, VoiceInfo};
#[cfg(any(target_os = "macos", test))]
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use tracing::{debug, info};

#[cfg(target_os = "macos")]
mod native;
#[cfg(any(target_os = "macos", test))]
mod stream;

fn macos_capabilities() -> EngineCapabilities {
    EngineCapabilities {
        acss: AcssCapabilities {
            rate: true,
            average_pitch: true,
            volume: true,
            ..AcssCapabilities::default()
        },
        audio_output: AudioOutputMode::StreamingPcm,
        cancellation: CancellationSupport::SynthesisAndPlayback,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities::default(),
        language_switching: true,
        text_repertoire: crate::contracts::TextRepertoire::Unicode,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

// Match the server registry's process-lifetime inventory. Native discovery and
// descriptor construction must not run again on the speech path. Restart the
// server after installing voices to refresh both Rust and bridge caches.
#[cfg(any(target_os = "macos", test))]
struct MacOsVoiceCache {
    load: fn() -> Vec<VoiceInfo>,
    voices: OnceLock<Vec<VoiceInfo>>,
    descriptor: OnceLock<EngineDescriptor>,
}

#[cfg(any(target_os = "macos", test))]
impl MacOsVoiceCache {
    const fn new(load: fn() -> Vec<VoiceInfo>) -> Self {
        Self {
            load,
            voices: OnceLock::new(),
            descriptor: OnceLock::new(),
        }
    }

    fn voices(&self) -> &[VoiceInfo] {
        self.voices.get_or_init(self.load)
    }

    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor
            .get_or_init(|| EngineDescriptor {
                id: "macos".to_owned(),
                display_name: "macOS AVSpeechSynthesizer".to_owned(),
                version: None,
                availability: Availability::Available,
                health: EngineHealth::Healthy,
                capabilities: macos_capabilities(),
                voices: self
                    .voices()
                    .iter()
                    .cloned()
                    .map(|voice| VoiceDescriptor::from_voice_info("macos", voice))
                    .collect(),
                espeak_variants: Vec::new(),
                default_voice_id: None,
            })
            .clone()
    }
}

#[cfg(target_os = "macos")]
static VOICE_CACHE: MacOsVoiceCache = MacOsVoiceCache::new(MacOsTtsEngine::load_voices);

// Keep the FFI layout and completion codes in sync with macos_bridge.m.
// Offsets use the bridge's monotonic clock, starting at omnivox_synthesize.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct SynthTimings {
    queue_wait_us: u64,
    write_started_us: u64,
    first_buffer_us: u64,
    last_buffer_us: u64,
    completion_signal_us: u64,
    capture_completed_us: u64,
    bridge_elapsed_us: u64,
    buffers_received: u32,
    completion_reason: u32,
}

#[cfg(any(target_os = "macos", test))]
impl SynthTimings {
    fn observed_us(offset: u64) -> Option<u64> {
        (offset != u64::MAX).then_some(offset)
    }

    fn first_buffer_to_return_us(&self) -> Option<u64> {
        self.bridge_elapsed_us
            .checked_sub(Self::observed_us(self.first_buffer_us)?)
    }

    fn last_buffer_to_completion_us(&self) -> Option<u64> {
        self.capture_completed_us
            .checked_sub(Self::observed_us(self.last_buffer_us)?)
    }

    fn completion_reason_name(&self) -> &'static str {
        completion_reason_name(self.completion_reason)
    }
}

#[cfg(any(target_os = "macos", test))]
fn completion_reason_name(reason: u32) -> &'static str {
    match reason {
        1 => "empty_buffer",
        2 => "inactivity_timeout", // Historical timing reports only.
        3 => "deadline",
        4 => "delegate_finished",
        5 => "cancelled",
        6 => "invalid_pcm",
        7 => "native_exception",
        8 => "voice_missing",
        9 => "pcm_limit",
        _ => "unknown",
    }
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

    fn load_voices() -> Vec<VoiceInfo> {
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

        debug!("Cached {} macOS voices for this process", voices.len());
        voices
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
        VOICE_CACHE.descriptor()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let mut collector = stream::Collector::default();
        native::synthesize(request, &mut collector)?;
        collector
            .result
            .ok_or_else(|| stream::failed("missing stream start"))
    }

    fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
        sink: &mut dyn crate::SynthesisStreamSink,
    ) -> Result<crate::SynthesisStreamCompletion, TtsError> {
        native::synthesize(request, sink)
    }

    fn stop(&self) {
        native::stop();
    }

    fn is_speaking(&self) -> bool {
        native::is_speaking()
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        VOICE_CACHE.voices().to_vec()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        VOICE_CACHE
            .voices()
            .iter()
            .find(|v| v.identifier == identifier || v.language == identifier)
            .cloned()
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
            espeak_variants: Vec::new(),
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

#[cfg(test)]
mod timing_tests {
    use super::SynthTimings;

    fn captured_audio() -> SynthTimings {
        SynthTimings {
            queue_wait_us: 100,
            write_started_us: 2_000,
            first_buffer_us: 10_000,
            last_buffer_us: 30_000,
            completion_signal_us: u64::MAX,
            capture_completed_us: 230_000,
            bridge_elapsed_us: 231_000,
            buffers_received: 4,
            completion_reason: 2,
        }
    }

    #[test]
    fn separates_audio_generation_from_inactivity_completion_wait() {
        let timings = captured_audio();
        assert_eq!(timings.first_buffer_to_return_us(), Some(221_000));
        assert_eq!(timings.last_buffer_to_completion_us(), Some(200_000));
        assert_eq!(timings.completion_reason_name(), "inactivity_timeout");
        assert_eq!(
            SynthTimings::observed_us(timings.completion_signal_us),
            None
        );

        let signalled = SynthTimings {
            completion_signal_us: 30_100,
            capture_completed_us: 30_200,
            bridge_elapsed_us: 31_000,
            completion_reason: 1,
            ..timings
        };
        assert_eq!(signalled.first_buffer_to_return_us(), Some(21_000));
        assert_eq!(signalled.last_buffer_to_completion_us(), Some(200));
        assert_eq!(signalled.completion_reason_name(), "empty_buffer");
        assert_eq!(
            SynthTimings::observed_us(signalled.completion_signal_us),
            Some(30_100)
        );
    }

    #[test]
    fn missing_callbacks_do_not_become_zero_latency_measurements() {
        let timings = SynthTimings {
            first_buffer_us: u64::MAX,
            last_buffer_us: u64::MAX,
            capture_completed_us: 30_000_000,
            bridge_elapsed_us: 30_000_100,
            buffers_received: 0,
            completion_reason: 3,
            ..captured_audio()
        };
        assert_eq!(timings.first_buffer_to_return_us(), None);
        assert_eq!(timings.last_buffer_to_completion_us(), None);
        assert_eq!(timings.completion_reason_name(), "deadline");
        assert_eq!(SynthTimings::observed_us(0), Some(0));
    }

    #[test]
    fn inconsistent_offsets_do_not_wrap_into_large_latency_values() {
        let timings = SynthTimings {
            bridge_elapsed_us: 5_000,
            capture_completed_us: 20_000,
            ..captured_audio()
        };
        assert_eq!(timings.first_buffer_to_return_us(), None);
        assert_eq!(timings.last_buffer_to_completion_us(), None);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::MacOsVoiceCache;
    use crate::{VoiceInfo, VoiceQuality};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    #[test]
    fn concurrent_inventory_and_descriptor_reads_discover_voices_once() {
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        let cache = MacOsVoiceCache::new(|| {
            LOADS.fetch_add(1, Ordering::SeqCst);
            vec![
                VoiceInfo {
                    identifier: "compact.en-US.Samantha".into(),
                    name: "Samantha".into(),
                    language: "en-US".into(),
                    quality: VoiceQuality::Compact,
                },
                VoiceInfo {
                    identifier: "enhanced.en-US.Samantha".into(),
                    name: "Samantha".into(),
                    language: "en-US".into(),
                    quality: VoiceQuality::Enhanced,
                },
            ]
        });
        let barrier = Barrier::new(8);
        std::thread::scope(|scope| {
            for reader in 0..8 {
                let cache = &cache;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..10 {
                        if reader % 2 == 0 {
                            // Exercise both orders of first access.
                            assert_eq!(cache.voices().len(), 2);
                        }
                        let mut descriptor = cache.descriptor();
                        assert_eq!(descriptor.id, "macos");
                        assert_eq!(descriptor.voices.len(), 2);
                        for (advertised, voice) in descriptor.voices.iter().zip(cache.voices()) {
                            assert_eq!(advertised.id.voice_id, voice.identifier);
                            assert_eq!(advertised.display_name, voice.name);
                            assert_eq!(
                                advertised.language.as_deref(),
                                Some(voice.language.as_str())
                            );
                        }
                        // Callers own their returned data, not the cached copy.
                        descriptor.voices.clear();
                        let mut voices = cache.voices().to_vec();
                        voices[0].name.clear();
                        assert_eq!(cache.voices()[0].name, "Samantha");
                        assert_eq!(cache.voices()[1].quality, VoiceQuality::Enhanced);
                    }
                });
            }
        });
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_inventory_does_not_trigger_repeated_native_discovery() {
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        let cache = MacOsVoiceCache::new(|| {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        });
        for _ in 0..10 {
            assert!(cache.voices().is_empty());
            assert!(cache.descriptor().voices.is_empty());
        }
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
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
