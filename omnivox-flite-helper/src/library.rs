//! Native file loading shared by legacy and managed startup.
use super::*;
use omnivox_tts::voice_library::FliteVoice as LibraryVoice;

// Caller holds FLITE_GLOBAL_STATE and takes ownership of the returned voice.
pub(super) fn load_external_voice(
    path: &Path,
    expected: Option<&LibraryVoice>,
) -> Result<NativeVoice, String> {
    let path = validate_external_voice_path(path)?;
    if let Some(expected) = expected {
        expected
            .file
            .open_verified()
            .map_err(|error| error.to_string())?;
    }
    let text = path
        .to_str()
        .ok_or_else(|| format!("Flite voice path is not valid Unicode: {}", path.display()))?;
    let path_string = CString::new(text)
        .map_err(|_| format!("Flite voice path contains a null byte: {}", path.display()))?;
    let pointer = unsafe { omnivox_flite_sys::omnivox_flite_load_voice(path_string.as_ptr()) };
    if pointer.is_null() {
        return Err(format!("Flite could not load voice {}", path.display()));
    }
    let name = match native_voice_name(pointer) {
        Ok(name) => name,
        Err(reason) => {
            unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(pointer) };
            return Err(format!("{}: {reason}", path.display()));
        }
    };
    let id = format!("flitevox:{name}");
    if expected.is_some_and(|expected| expected.physical_id != id) {
        unsafe { omnivox_flite_sys::omnivox_flite_delete_voice(pointer) };
        return Err("Flite native voice ID differs from the validated library".to_owned());
    }
    Ok(NativeVoice {
        pointer,
        id,
        name,
        owned: true,
    })
}

#[cfg(test)]
mod tests;
