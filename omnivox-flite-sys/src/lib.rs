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

pub type FliteStreamCallback = unsafe extern "C" fn(
    samples: *const c_short,
    sample_count: c_int,
    sample_rate: c_int,
    channel_count: c_int,
    last: c_int,
    markers: *const FliteWordMarker,
    marker_count: c_int,
    user_data: *mut c_void,
) -> c_int;

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
    pub fn omnivox_flite_synthesize_stream(
        voice: *mut FliteVoice,
        text: *const c_char,
        duration_stretch: c_float,
        f0_shift: c_float,
        marker_capacity: c_int,
        callback: FliteStreamCallback,
        user_data: *mut c_void,
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
    use std::sync::Mutex;

    static FLITE_TEST_STATE: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct StreamCapture {
        chunks: usize,
        samples: usize,
        markers: Vec<FliteWordMarker>,
        saw_last: bool,
    }

    unsafe extern "C" fn capture_stream(
        samples: *const c_short,
        sample_count: c_int,
        sample_rate: c_int,
        channel_count: c_int,
        last: c_int,
        markers: *const FliteWordMarker,
        marker_count: c_int,
        user_data: *mut c_void,
    ) -> c_int {
        assert!(!user_data.is_null());
        assert!(sample_count >= 0);
        assert_eq!(sample_rate, 16_000);
        assert_eq!(channel_count, 1);
        let capture = unsafe { &mut *user_data.cast::<StreamCapture>() };
        if sample_count > 0 {
            assert!(!samples.is_null());
            capture.chunks += 1;
            capture.samples += sample_count as usize;
        }
        assert!(marker_count >= 0);
        if marker_count > 0 {
            assert!(!markers.is_null());
            capture.markers.extend_from_slice(unsafe {
                std::slice::from_raw_parts(markers, marker_count as usize)
            });
        }
        capture.saw_last |= last != 0;
        1
    }

    #[test]
    fn bundled_slt_voice_synthesizes_pcm() {
        let _state = FLITE_TEST_STATE.lock().unwrap();
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

    #[test]
    fn bundled_slt_voice_streams_pcm_and_word_markers() {
        let _state = FLITE_TEST_STATE.lock().unwrap();
        unsafe {
            assert_eq!(omnivox_flite_initialize(), 0);
            let voice = omnivox_flite_register_slt();
            assert!(!voice.is_null());
            let text = CString::new("Flite streams its markers.").unwrap();
            let mut capture = StreamCapture::default();

            let synthesis = omnivox_flite_synthesize_stream(
                voice,
                text.as_ptr(),
                1.0,
                1.0,
                8,
                capture_stream,
                std::ptr::from_mut(&mut capture).cast(),
            );

            assert!(!synthesis.is_null());
            assert!(capture.chunks > 1);
            assert!(capture.samples > 1_000);
            assert_eq!(
                capture.samples,
                omnivox_flite_synthesis_sample_count(synthesis) as usize
            );
            assert!(capture.saw_last);
            assert_eq!(capture.markers.len(), 4);
            assert_eq!(capture.markers[0].text_start, 0);
            assert_eq!(capture.markers[1].text_start, 6);
            assert_eq!(capture.markers[2].text_start, 14);
            assert_eq!(capture.markers[3].text_start, 18);
            omnivox_flite_delete_synthesis(synthesis);
        }
    }
}
