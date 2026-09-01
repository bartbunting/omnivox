//! Minimal FFI boundary for Omnivox's pinned Flite v2.2 companion.
//!
//! The linked Flite source remains under its upstream BSD-like licence. The
//! wrapper keeps Flite structs opaque to the Rust adapter.

use std::ffi::{c_char, c_float, c_int, c_short, c_void};

pub const FLITE_VERSION: &str = "2.2";
pub const FLITE_COMMIT: &str = "e9e2e37c329dbe98bfeb27a1828ef9a71fa84f88";

pub type FliteVoice = c_void;
pub type FliteWave = c_void;

unsafe extern "C" {
    pub fn omnivox_flite_initialize() -> c_int;
    pub fn omnivox_flite_register_slt() -> *mut FliteVoice;
    pub fn omnivox_flite_load_voice(path: *const c_char) -> *mut FliteVoice;
    pub fn omnivox_flite_delete_voice(voice: *mut FliteVoice);
    pub fn omnivox_flite_voice_name(voice: *const FliteVoice) -> *const c_char;
    pub fn omnivox_flite_synthesize(
        voice: *mut FliteVoice,
        text: *const c_char,
        duration_stretch: c_float,
        f0_shift: c_float,
    ) -> *mut FliteWave;
    pub fn omnivox_flite_wave_sample_rate(wave: *const FliteWave) -> c_int;
    pub fn omnivox_flite_wave_sample_count(wave: *const FliteWave) -> c_int;
    pub fn omnivox_flite_wave_channel_count(wave: *const FliteWave) -> c_int;
    pub fn omnivox_flite_wave_samples(wave: *const FliteWave) -> *const c_short;
    pub fn omnivox_flite_delete_wave(wave: *mut FliteWave);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn bundled_slt_voice_synthesizes_pcm() {
        unsafe {
            assert_eq!(omnivox_flite_initialize(), 0);
            let voice = omnivox_flite_register_slt();
            assert!(!voice.is_null());
            assert_eq!(
                CStr::from_ptr(omnivox_flite_voice_name(voice))
                    .to_str()
                    .unwrap(),
                "cmu_us_slt"
            );

            let text = CString::new("Flite is ready.").unwrap();
            let wave = omnivox_flite_synthesize(voice, text.as_ptr(), 1.0, 1.0);
            assert!(!wave.is_null());
            assert_eq!(omnivox_flite_wave_sample_rate(wave), 16_000);
            assert_eq!(omnivox_flite_wave_channel_count(wave), 1);
            assert!(omnivox_flite_wave_sample_count(wave) > 1_000);
            assert!(!omnivox_flite_wave_samples(wave).is_null());
            omnivox_flite_delete_wave(wave);
        }
    }
}
