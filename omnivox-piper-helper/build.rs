fn main() {
    // omnivox-piper-sys exposes the directory containing piper-phonemize,
    // ONNX Runtime, and its espeak-ng through Cargo metadata. Embed that path
    // only in the helper executable that actually loads those libraries.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "linux" | "macos") {
        let Ok(rpath) = std::env::var("DEP_PIPER_RPATH") else {
            return;
        };
        if !rpath.is_empty() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{rpath}");
        }
        let adjacent = if target_os == "macos" {
            "@loader_path"
        } else {
            "$ORIGIN"
        };
        println!("cargo:rustc-link-arg=-Wl,-rpath,{adjacent}");
    }
}
