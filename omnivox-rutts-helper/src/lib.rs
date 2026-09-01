//! RuTTS v6.3.3 adapter for the isolated Omnivox companion.

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use omnivox_tts::contracts::{
    buffered_post_synthesis_dimensions, AcssCapabilities, AudioOutputMode, Availability,
    CancellationSupport, ConcurrencyModel, EngineCapabilities, EngineDescriptor, EngineHealth,
    MarkerCapabilities, PhysicalVoiceId, TextRepertoire, VoiceDescriptor, VoiceGender,
};
use omnivox_tts::helper_protocol::MAX_HELPER_SYNTHESIS_BYTES;
use omnivox_tts::rate_calibration::interpolate;
use omnivox_tts::{
    AudioBuffer, SynthesisCancellationToken, SynthesisRequest, SynthesisResult, TtsEngine,
    TtsError, VoiceInfo, VoiceQuality,
};

const ENGINE_ID: &str = "rutts";
const MALE_VOICE_ID: &str = "male";
const FEMALE_VOICE_ID: &str = "female";
const MAX_NATIVE_SAMPLES: usize = MAX_HELPER_SYNTHESIS_BYTES / std::mem::size_of::<i16>();

/// Serialized access to RuTTS's built-in male and female voices. Native
/// synthesis remains isolated in the helper process.
pub struct RuttsTtsEngine {
    descriptor: EngineDescriptor,
    runtime: Mutex<()>,
    cancellation: AtomicBool,
    speaking: AtomicBool,
}

impl RuttsTtsEngine {
    pub fn new() -> Self {
        Self {
            descriptor: descriptor(),
            runtime: Mutex::new(()),
            cancellation: AtomicBool::new(false),
            speaking: AtomicBool::new(false),
        }
    }

    fn runtime(&self) -> Result<MutexGuard<'_, ()>, TtsError> {
        self.runtime.lock().map_err(|error| {
            TtsError::SynthesisFailed(format!("RuTTS state lock is poisoned: {error}"))
        })
    }
}

impl Default for RuttsTtsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsEngine for RuttsTtsEngine {
    fn descriptor(&self) -> EngineDescriptor {
        self.descriptor.clone()
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<SynthesisResult, TtsError> {
        let voice_id = request.voice_id_for_engine(ENGINE_ID)?;
        let alternative_voice = match voice_id {
            MALE_VOICE_ID => false,
            FEMALE_VOICE_ID => true,
            _ => return Err(TtsError::VoiceNotFound(voice_id.to_owned())),
        };
        let actual_voice = Some(PhysicalVoiceId::new(ENGINE_ID, voice_id));
        if request.text.is_empty() {
            return Ok(SynthesisResult::audio(
                ENGINE_ID,
                actual_voice,
                AudioBuffer::empty(),
            ));
        }

        let text = encode_koi8_r(&request.text)?;
        let _runtime = self.runtime()?;
        self.cancellation.store(false, Ordering::Release);
        self.speaking.store(true, Ordering::Release);
        let _speaking = SpeakingGuard(&self.speaking);
        let mut capture = Capture {
            samples: Vec::new(),
            cancellation: &self.cancellation,
            request_cancellation: request.cancellation.as_ref(),
            volume: request.settings.volume.clamp(0.0, 1.0),
            failure: None,
        };
        let status = unsafe {
            omnivox_rutts_sys::omnivox_rutts_synthesize(
                text.as_ptr().cast(),
                map_rate(request.settings.rate),
                map_pitch(request.settings.pitch),
                map_intonation(request.normalized_acss.pitch_range),
                i32::from(alternative_voice),
                consume_pcm,
                std::ptr::from_mut(&mut capture).cast(),
            )
        };
        if let Some(failure) = capture.failure {
            return Err(TtsError::SynthesisFailed(failure.to_owned()));
        }
        if capture.cancelled() || status > 0 {
            return Err(TtsError::SynthesisFailed(
                "RuTTS synthesis was cancelled".to_owned(),
            ));
        }
        if status < 0 {
            return Err(TtsError::SynthesisFailed(format!(
                "RuTTS rejected the synthesis call with status {status}"
            )));
        }
        if capture.samples.is_empty() {
            return Err(TtsError::SynthesisFailed(
                "RuTTS returned no PCM".to_owned(),
            ));
        }

        let audio = AudioBuffer::try_from_interleaved_i16(
            &capture.samples,
            omnivox_rutts_sys::RUTTS_SAMPLE_RATE,
            1,
        )
        .map_err(|error| {
            TtsError::SynthesisFailed(format!("could not canonicalize RuTTS PCM: {error}"))
        })?;
        let mut result = SynthesisResult::audio(ENGINE_ID, actual_voice, audio);
        result.degraded_acss = request
            .normalized_acss
            .clone()
            .degrade_for(&capabilities().acss)
            .omitted;
        Ok(result)
    }

    fn stop(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Acquire)
    }

    fn available_voices(&self) -> Vec<VoiceInfo> {
        self.descriptor
            .voices
            .iter()
            .map(|voice| VoiceInfo {
                identifier: voice.id.voice_id.clone(),
                name: voice.display_name.clone(),
                language: voice.language.clone().unwrap_or_default(),
                quality: voice.quality,
            })
            .collect()
    }

    fn voice_info(&self, identifier: &str) -> Option<VoiceInfo> {
        self.available_voices()
            .into_iter()
            .find(|voice| voice.identifier == identifier || voice.name == identifier)
    }
}

struct SpeakingGuard<'a>(&'a AtomicBool);

impl Drop for SpeakingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Capture<'a> {
    samples: Vec<i16>,
    cancellation: &'a AtomicBool,
    request_cancellation: Option<&'a SynthesisCancellationToken>,
    volume: f32,
    failure: Option<&'static str>,
}

impl Capture<'_> {
    fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
            || self
                .request_cancellation
                .is_some_and(SynthesisCancellationToken::is_cancelled)
    }
}

unsafe extern "C" fn consume_pcm(
    samples: *const i8,
    count: usize,
    user_data: *mut c_void,
) -> c_int {
    if user_data.is_null() {
        return 1;
    }
    let capture = unsafe { &mut *user_data.cast::<Capture<'_>>() };
    if capture.cancelled() {
        return 1;
    }
    if count == 0 {
        return 0;
    }
    if samples.is_null() {
        capture.failure = Some("RuTTS returned a null PCM buffer");
        return 1;
    }
    if count > MAX_NATIVE_SAMPLES.saturating_sub(capture.samples.len()) {
        capture.failure = Some("RuTTS PCM exceeds the helper limit");
        return 1;
    }
    if capture.samples.try_reserve(count).is_err() {
        capture.failure = Some("could not allocate the RuTTS PCM buffer");
        return 1;
    }

    let native = unsafe { std::slice::from_raw_parts(samples, count) };
    capture.samples.extend(native.iter().map(|sample| {
        let expanded = i32::from(*sample) * 256;
        (expanded as f32 * capture.volume).round() as i16
    }));
    i32::from(capture.cancelled())
}

fn capabilities() -> EngineCapabilities {
    EngineCapabilities {
        acss: AcssCapabilities {
            rate: true,
            average_pitch: true,
            pitch_range: true,
            volume: true,
            ..AcssCapabilities::default()
        },
        audio_output: AudioOutputMode::BufferedPcm,
        cancellation: CancellationSupport::SynthesisAndPlayback,
        concurrency: ConcurrencyModel::Serialized,
        markers: MarkerCapabilities::default(),
        language_switching: false,
        text_repertoire: TextRepertoire::Koi8R,
        post_synthesis_dimensions: buffered_post_synthesis_dimensions(),
        native_extensions: Vec::new(),
    }
}

fn descriptor() -> EngineDescriptor {
    let voices = vec![
        VoiceDescriptor {
            id: PhysicalVoiceId::new(ENGINE_ID, MALE_VOICE_ID),
            display_name: "RuTTS Male".to_owned(),
            language: Some("ru-RU".to_owned()),
            gender: Some(VoiceGender::Male),
            quality: VoiceQuality::Compact,
            availability: Availability::Available,
        },
        VoiceDescriptor {
            id: PhysicalVoiceId::new(ENGINE_ID, FEMALE_VOICE_ID),
            display_name: "RuTTS Female".to_owned(),
            language: Some("ru-RU".to_owned()),
            gender: Some(VoiceGender::Female),
            quality: VoiceQuality::Compact,
            availability: Availability::Available,
        },
    ];
    EngineDescriptor {
        id: ENGINE_ID.to_owned(),
        display_name: "RuTTS".to_owned(),
        version: Some(format!(
            "{} ({})",
            omnivox_rutts_sys::RUTTS_VERSION,
            &omnivox_rutts_sys::RUTTS_COMMIT[..12]
        )),
        availability: Availability::Available,
        health: EngineHealth::Healthy,
        capabilities: capabilities(),
        default_voice_id: Some(MALE_VOICE_ID.to_owned()),
        voices,
    }
}

fn map_rate(rate: f32) -> i32 {
    // Russian reference and saturation policy: docs/RATE-CALIBRATION.md.
    const CALIBRATION: &[(f32, f32)] = &[
        (0.0, 67.697_849),
        (0.1, 78.578_08),
        (0.2, 96.268_38),
        (0.3, 111.978_17),
        (0.4, 132.210_04),
        (0.5, 152.467_25),
        (0.6, 179.160_37),
        (0.7, 209.018_16),
        (0.8, 300.603_45),
        (0.9, 393.356_38),
        (1.0, 478.062_56),
        (1.2, 500.000_000),
    ];
    interpolate(rate, CALIBRATION).round() as i32
}

fn map_pitch(pitch: f32) -> i32 {
    let pitch = pitch.clamp(0.5, 2.0);
    if pitch <= 1.0 {
        (50.0 + (pitch - 0.5) * 100.0).round() as i32
    } else {
        (100.0 + (pitch - 1.0) * 200.0).round() as i32
    }
}

fn map_intonation(pitch_range: Option<f32>) -> i32 {
    let Some(value) = pitch_range else {
        return 100;
    };
    let value = value.clamp(0.0, 1.0);
    if value <= 0.5 {
        (value * 200.0).round() as i32
    } else {
        (100.0 + (value - 0.5) * 80.0).round() as i32
    }
}

fn encode_koi8_r(text: &str) -> Result<Vec<u8>, TtsError> {
    let mut encoded = Vec::with_capacity(text.len().saturating_add(1));
    for (offset, character) in text.char_indices() {
        let byte = match character {
            '\0' => {
                return Err(TtsError::InvalidParameter(
                    "RuTTS text contains a null byte".to_owned(),
                ));
            }
            character if character.is_ascii() => character as u8,
            '\u{0451}' => 0xa3,
            '\u{0401}' => 0xb3,
            '\u{044e}'..='\u{044f}' | '\u{0430}'..='\u{044d}' => {
                KOI8_R_LOWER[(u32::from(character) - u32::from('\u{0430}')) as usize]
            }
            '\u{042e}'..='\u{042f}' | '\u{0410}'..='\u{042d}' => {
                KOI8_R_UPPER[(u32::from(character) - u32::from('\u{0410}')) as usize]
            }
            _ => {
                return Err(TtsError::InvalidParameter(format!(
                    "RuTTS cannot encode {character:?} at UTF-8 byte offset {offset} as KOI8-R"
                )));
            }
        };
        encoded.push(byte);
    }
    encoded.push(0);
    Ok(encoded)
}

const KOI8_R_LOWER: [u8; 32] = [
    0xc1, 0xc2, 0xd7, 0xc7, 0xc4, 0xc5, 0xd6, 0xda, 0xc9, 0xca, 0xcb, 0xcc, 0xcd, 0xce, 0xcf, 0xd0,
    0xd2, 0xd3, 0xd4, 0xd5, 0xc6, 0xc8, 0xc3, 0xde, 0xdb, 0xdd, 0xdf, 0xd9, 0xd8, 0xdc, 0xc0, 0xd1,
];
const KOI8_R_UPPER: [u8; 32] = [
    0xe1, 0xe2, 0xf7, 0xe7, 0xe4, 0xe5, 0xf6, 0xfa, 0xe9, 0xea, 0xeb, 0xec, 0xed, 0xee, 0xef, 0xf0,
    0xf2, 0xf3, 0xf4, 0xf5, 0xe6, 0xe8, 0xe3, 0xfe, 0xfb, 0xfd, 0xff, 0xf9, 0xf8, 0xfc, 0xe0, 0xf1,
];

#[cfg(test)]
mod tests {
    use super::*;
    use omnivox_tts::{SynthesisCancellationToken, TtsSettings};

    fn request(voice: &str) -> SynthesisRequest {
        SynthesisRequest::new(
            "Привет, мир!",
            TtsSettings {
                voice: voice.to_owned(),
                ..TtsSettings::default()
            },
        )
    }

    #[test]
    fn koi8_r_encoding_matches_the_upstream_input_contract() {
        assert_eq!(
            encode_koi8_r("Привет, Ёж.").unwrap(),
            b"\xf0\xd2\xc9\xd7\xc5\xd4, \xb3\xd6.\0"
        );
        assert!(encode_koi8_r("Привет — мир").is_err());
        assert!(encode_koi8_r("nul\0byte").is_err());
    }

    #[test]
    fn parameter_mappings_preserve_calibrated_rate_and_other_bounds() {
        assert_eq!(
            (map_rate(0.0), map_rate(0.5), map_rate(2.0)),
            (68, 152, 500)
        );
        let mapped: Vec<_> = (0..=20)
            .map(|point| map_rate(point as f32 / 10.0))
            .collect();
        assert!(mapped.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(
            (map_pitch(0.5), map_pitch(1.0), map_pitch(2.0)),
            (50, 100, 300)
        );
        assert_eq!(
            (
                map_intonation(Some(0.0)),
                map_intonation(Some(0.5)),
                map_intonation(Some(1.0)),
                map_intonation(None),
            ),
            (0, 100, 140, 100)
        );
    }

    #[test]
    fn engine_exposes_and_synthesizes_both_built_in_voices() {
        let engine = RuttsTtsEngine::new();
        let descriptor = engine.descriptor();
        assert_eq!(descriptor.default_voice_id.as_deref(), Some(MALE_VOICE_ID));
        assert_eq!(descriptor.voices.len(), 2);
        assert_eq!(
            descriptor.capabilities.text_repertoire,
            TextRepertoire::Koi8R
        );

        let male = engine.synthesize(&request(MALE_VOICE_ID)).unwrap();
        let female = engine.synthesize(&request(FEMALE_VOICE_ID)).unwrap();
        assert!(!male.audio.is_empty());
        assert!(!female.audio.is_empty());
        assert_ne!(male.audio.samples, female.audio.samples);
    }

    #[test]
    fn request_cancellation_stops_native_pcm_collection() {
        let engine = RuttsTtsEngine::new();
        let cancellation = SynthesisCancellationToken::new();
        cancellation.cancel();
        let request = request(MALE_VOICE_ID).with_cancellation(cancellation);

        let error = engine.synthesize(&request).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert!(!engine.is_speaking());
    }
}
