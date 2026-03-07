fn main() {
    // When the piper feature is enabled, omnivox-piper-sys exposes the path
    // to its dynamic libraries (piper_phonemize, onnxruntime, espeak-ng) via
    // DEP_PIPER_RPATH.  Embed that path as an rpath in the final binary so
    // the dynamic linker can find the libs without DYLD_LIBRARY_PATH.
    if let Ok(rpath) = std::env::var("DEP_PIPER_RPATH") {
        if !rpath.is_empty() {
            // macOS / Linux: embed rpath into the final omnivox binary.
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", rpath);
        }
    }
}
