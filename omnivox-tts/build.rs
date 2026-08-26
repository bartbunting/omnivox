use std::path::PathBuf;

fn main() {
    // Compile the Objective-C bridge for macOS buffer capture.
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

    // Cargo can run this build script before the regular espeak-rs-sys
    // dependency has generated its data, so searching sibling OUT_DIRs here is
    // racy on a clean build and can select stale data on an incremental build.
    // The supported build entry point stages the completed dependency output
    // in the Cargo profile directory. Embed that stable parent path; runtime
    // discovery also checks beside a relocated executable before using it.
    let staged_parent = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .and_then(|out_dir| {
            out_dir
                .ancestors()
                .find(|path| path.file_name().is_some_and(|name| name == "build"))
                .and_then(|build_dir| build_dir.parent())
                .map(PathBuf::from)
        });

    println!(
        "cargo:rustc-env=ESPEAK_NG_DATA_DIR={}",
        staged_parent
            .as_deref()
            .map_or_else(String::new, |path| path.display().to_string())
    );
}
