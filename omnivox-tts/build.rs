use std::path::PathBuf;

fn main() {
    // Compile the Objective-C bridge for macOS buffer capture
    #[cfg(target_os = "macos")]
    {
        cc::Build::new()
            .file("src/macos_bridge.m")
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .compile("macos_bridge");

        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
    // Find the espeak-ng data directory from the espeak-rs-sys build output.
    // The espeak-rs-sys crate compiles espeak-ng from source and places the
    // data files under OUT_DIR/share/espeak-ng-data/
    //
    // We need to pass the parent directory (containing espeak-ng-data/) to
    // espeak_Initialize at runtime. Since espeak-rs-sys doesn't export this
    // path via cargo metadata, we search for it in the target build directory.

    let out_dir = std::env::var("OUT_DIR").unwrap_or_default();

    // Walk up from our OUT_DIR to find the build directory
    // Our OUT_DIR is like: target/debug/build/omnivox-tts-HASH/out
    // espeak-rs-sys data is at: target/debug/build/espeak-rs-sys-HASH/out/share/espeak-ng-data
    if let Some(build_dir) = PathBuf::from(&out_dir)
        .ancestors()
        .find(|p| p.file_name().map_or(false, |n| n == "build"))
    {
        // Search for espeak-ng-data in the build directory
        if let Ok(entries) = std::fs::read_dir(build_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .map_or(false, |n| n.to_string_lossy().starts_with("espeak-rs-sys-"))
                {
                    let data_path = path.join("out").join("share");
                    let phontab = data_path.join("espeak-ng-data").join("phontab");
                    if phontab.exists() {
                        println!(
                            "cargo:rustc-env=ESPEAK_NG_DATA_DIR={}",
                            data_path.display()
                        );
                        return;
                    }
                }
            }
        }
    }

    // If we can't find it in the build dir, check system paths
    let system_paths = [
        "/usr/share",
        "/usr/local/share",
        "/opt/homebrew/share",
    ];
    for base in &system_paths {
        let data_path = PathBuf::from(base).join("espeak-ng-data").join("phontab");
        if data_path.exists() {
            println!("cargo:rustc-env=ESPEAK_NG_DATA_DIR={}", base);
            return;
        }
    }

    // Fallback: let espeak-ng try its default paths
    println!("cargo:rustc-env=ESPEAK_NG_DATA_DIR=");
}
