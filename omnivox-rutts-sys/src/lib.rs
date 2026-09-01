//! Minimal native boundary for Omnivox's pinned RuTTS v6.3.3 companion.
//!
//! The linked RuTTS source and its built-in male and female voices remain
//! under the upstream MIT licence. RuLex is not compiled or loaded.

use std::ffi::{c_char, c_int, c_void};

pub const RUTTS_VERSION: &str = "6.3.3";
pub const RUTTS_COMMIT: &str = "2848d2892097320ed37fc963b439b15803f47f0c";
pub const RUTTS_SAMPLE_RATE: u32 = 10_000;

pub type RuttsCallback =
    unsafe extern "C" fn(samples: *const i8, count: usize, user_data: *mut c_void) -> c_int;

unsafe extern "C" {
    pub fn omnivox_rutts_synthesize(
        koi8r_text: *const c_char,
        speech_rate: c_int,
        voice_pitch: c_int,
        intonation: c_int,
        alternative_voice: c_int,
        callback: RuttsCallback,
        user_data: *mut c_void,
    ) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Capture {
        samples: Vec<i8>,
        callback_count: usize,
        cancel: bool,
    }

    unsafe extern "C" fn capture(
        samples: *const i8,
        count: usize,
        user_data: *mut c_void,
    ) -> c_int {
        let capture = unsafe { &mut *user_data.cast::<Capture>() };
        capture.callback_count += 1;
        capture
            .samples
            .extend_from_slice(unsafe { std::slice::from_raw_parts(samples, count) });
        i32::from(capture.cancel)
    }

    fn synthesize(capture: &mut Capture, alternative_voice: bool) -> c_int {
        // "Привет мир." in KOI8-R, followed by the required null terminator.
        let text = b"\xf0\xd2\xc9\xd7\xc5\xd4 \xcd\xc9\xd2.\0";
        unsafe {
            omnivox_rutts_synthesize(
                text.as_ptr().cast(),
                100,
                100,
                100,
                i32::from(alternative_voice),
                self::capture,
                std::ptr::from_mut(capture).cast(),
            )
        }
    }

    #[test]
    fn male_voice_synthesizes_signed_eight_bit_pcm() {
        let mut capture = Capture::default();

        assert_eq!(synthesize(&mut capture, false), 0);
        assert!(capture.samples.len() > 1_000);
        assert!(capture.samples.iter().any(|sample| *sample != 0));
        assert!(capture.callback_count > 1);
    }

    #[test]
    fn female_voice_synthesizes_distinct_pcm() {
        let mut male = Capture::default();
        let mut female = Capture::default();

        assert_eq!(synthesize(&mut male, false), 0);
        assert_eq!(synthesize(&mut female, true), 0);
        assert!(female.samples.len() > 1_000);
        assert!(female.samples.iter().any(|sample| *sample != 0));
        assert_ne!(female.samples, male.samples);
    }

    #[test]
    fn cancellation_suppresses_late_upstream_pcm() {
        let mut capture = Capture {
            cancel: true,
            ..Capture::default()
        };

        assert_eq!(synthesize(&mut capture, false), 1);
        assert_eq!(capture.callback_count, 1);
        assert_eq!(capture.samples.len(), 4_096);
    }
}
