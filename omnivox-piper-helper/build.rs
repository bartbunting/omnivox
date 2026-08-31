fn main() {
    // omnivox-piper-sys exposes the directory containing libpiper and ONNX
    // Runtime through Cargo metadata. Developer builds can use that absolute
    // path; staged companion builds must rely only on the adjacent runtime.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "linux" | "macos") {
        let relocatable = std::env::var("OMNIVOX_PIPER_RELOCATABLE").as_deref() == Ok("1");
        if !relocatable {
            let Ok(rpath) = std::env::var("DEP_PIPER_RPATH") else {
                return;
            };
            if !rpath.is_empty() {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{rpath}");
            }
        }
        let adjacent = if target_os == "macos" {
            "@loader_path"
        } else {
            "$ORIGIN"
        };
        println!("cargo:rustc-link-arg=-Wl,-rpath,{adjacent}");
    }
    println!("cargo:rerun-if-env-changed=OMNIVOX_PIPER_RELOCATABLE");
}
