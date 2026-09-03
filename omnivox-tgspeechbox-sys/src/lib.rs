//! Narrow native boundary for the pinned TGSpeechBox companion.

use std::ffi::{c_char, c_double, c_int, c_short, c_void};

pub const TGSPEECHBOX_RELEASE: &str = "v-310@f5ec247";
pub const TGSPEECHBOX_COMMIT: &str = "f5ec247bca50507ab1e2ed661136395538dc3e97";
pub const TGSPEECHBOX_DSP_VERSION: u32 = 8;
pub const TGSPEECHBOX_FRONTEND_ABI_VERSION: i32 = 5;

unsafe extern "C" {
    pub fn omnivox_tgspeechbox_create(pack_root: *const c_char, sample_rate: c_int) -> *mut c_void;
    pub fn omnivox_tgspeechbox_create_error() -> *const c_char;
    pub fn omnivox_tgspeechbox_destroy(handle: *mut c_void);
    pub fn omnivox_tgspeechbox_last_error(handle: *mut c_void) -> *const c_char;
    pub fn omnivox_tgspeechbox_dsp_version() -> u32;
    pub fn omnivox_tgspeechbox_frontend_abi_version() -> c_int;
    pub fn omnivox_tgspeechbox_languages(handle: *mut c_void) -> *mut c_char;
    pub fn omnivox_tgspeechbox_profile_names(handle: *mut c_void) -> *const c_char;
    pub fn omnivox_tgspeechbox_free_string(value: *mut c_char);
    pub fn omnivox_tgspeechbox_configure(
        handle: *mut c_void,
        language: *const c_char,
        profile: *const c_char,
    ) -> c_int;
    pub fn omnivox_tgspeechbox_prepare_text(
        handle: *mut c_void,
        text: *const c_char,
    ) -> *mut c_char;
    pub fn omnivox_tgspeechbox_begin(
        handle: *mut c_void,
        text: *const c_char,
        ipa: *const c_char,
        speed: c_double,
        base_pitch_hz: c_double,
        inflection: c_double,
        volume: c_double,
    ) -> c_int;
    pub fn omnivox_tgspeechbox_next(
        handle: *mut c_void,
        samples: *mut c_short,
        capacity: usize,
    ) -> c_int;
    pub fn omnivox_tgspeechbox_reset(handle: *mut c_void) -> c_int;
}
