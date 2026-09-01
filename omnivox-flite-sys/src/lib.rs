//! Minimal FFI boundary for Omnivox's pinned Flite v2.2 companion.
//!
//! The linked Flite source remains under its upstream BSD-like licence. The
//! wrapper keeps Flite structs opaque to the Rust adapter.

use std::ffi::{c_char, c_float, c_int, c_short, c_void};

pub const FLITE_VERSION: &str = "2.2";
pub const FLITE_COMMIT: &str = "e9e2e37c329dbe98bfeb27a1828ef9a71fa84f88";

pub type FliteVoice = c_void;
pub type FliteSynthesis = c_void;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FliteWordMarker {
    pub frame_offset: c_int,
    pub text_start: c_int,
    pub text_length: c_int,
}

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
    ) -> *mut FliteSynthesis;
    pub fn omnivox_flite_synthesis_sample_rate(synthesis: *mut FliteSynthesis) -> c_int;
    pub fn omnivox_flite_synthesis_sample_count(synthesis: *mut FliteSynthesis) -> c_int;
    pub fn omnivox_flite_synthesis_channel_count(synthesis: *mut FliteSynthesis) -> c_int;
    pub fn omnivox_flite_synthesis_samples(synthesis: *mut FliteSynthesis) -> *const c_short;
    pub fn omnivox_flite_synthesis_word_markers(
        synthesis: *mut FliteSynthesis,
        text: *const c_char,
        markers: *mut FliteWordMarker,
        capacity: c_int,
    ) -> c_int;
    pub fn omnivox_flite_delete_synthesis(synthesis: *mut FliteSynthesis);
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
            let synthesis = omnivox_flite_synthesize(voice, text.as_ptr(), 1.0, 1.0);
            assert!(!synthesis.is_null());
            assert_eq!(omnivox_flite_synthesis_sample_rate(synthesis), 16_000);
            assert_eq!(omnivox_flite_synthesis_channel_count(synthesis), 1);
            assert!(omnivox_flite_synthesis_sample_count(synthesis) > 1_000);
            assert!(!omnivox_flite_synthesis_samples(synthesis).is_null());

            let mut markers = [FliteWordMarker::default(); 8];
            let marker_count = omnivox_flite_synthesis_word_markers(
                synthesis,
                text.as_ptr(),
                markers.as_mut_ptr(),
                markers.len() as c_int,
            );
            assert_eq!(marker_count, 3);
            assert_eq!(markers[0].text_start, 0);
            assert_eq!(markers[0].text_length, 5);
            assert_eq!(markers[1].text_start, 6);
            assert_eq!(markers[1].text_length, 2);
            assert_eq!(markers[2].text_start, 9);
            assert_eq!(markers[2].text_length, 5);
            assert!(markers[..marker_count as usize]
                .windows(2)
                .all(|pair| pair[0].frame_offset <= pair[1].frame_offset));

            let mut undersized = [FliteWordMarker::default(); 2];
            assert_eq!(
                omnivox_flite_synthesis_word_markers(
                    synthesis,
                    text.as_ptr(),
                    undersized.as_mut_ptr(),
                    undersized.len() as c_int,
                ),
                -1
            );
            omnivox_flite_delete_synthesis(synthesis);
        }
    }
}
