# GitHub Actions CI/CD

## Overview

This repository uses GitHub Actions to automatically build omnivox binaries for multiple platforms on every push to `main`.

## Workflows

### build.yml

The main build workflow that:

1. **Builds** omnivox for 6 target platforms:
   - Windows x64 (x86_64-pc-windows-msvc)
   - Windows ARM64 (aarch64-pc-windows-msvc)
   - Linux x64 (x86_64-unknown-linux-gnu)
   - Linux ARM64 (aarch64-unknown-linux-gnu)
   - macOS x64 Intel (x86_64-apple-darwin)
   - macOS ARM64 Apple Silicon (aarch64-apple-darwin)

2. **Tests** the code on Linux, Windows, and macOS (native architecture)

3. **Creates releases** with all binaries packaged as:
   - Windows: `.zip` archives
   - Linux/macOS: `.tar.gz` tarballs

## Build Process

Each platform build:

1. Checks out the code
2. Installs Rust with the target triple
3. Installs platform-specific build dependencies
4. Builds with `cargo build --locked --release --target <triple>` using the
   exact Rust release in `rust-toolchain.toml`
5. Uploads the binary as an artifact

## Platform-Specific Details

### Windows
- Uses Visual Studio build tools (pre-installed on windows-latest runner)
- ARM64 builds use cross-compilation (native MSVC toolchain)
- Compiles WinRT SpeechSynthesizer backend via windows-rs crate

### Linux
- Installs gcc, g++, pkg-config, autoconf, automake, libtool
- ARM64 cross-compilation uses gcc-aarch64-linux-gnu
- espeak-ng compiled from source (bundled via espeak-rs-sys)

### macOS
- Uses Xcode toolchain (pre-installed on macos-latest runner)
- x64 and ARM64 use native cross-compilation (Apple Clang supports both)
- ObjC bridge compiled via cc crate (build.rs)
- Links against AVFoundation and Foundation frameworks

## Release Tagging

Releases are automatically created with tags in the format:
```
build-YYYYMMDD-HHMMSS
```

Example: `build-20260209-143022`

## Artifacts

Build artifacts are retained for 90 days (GitHub default) and can be downloaded from:
- Individual workflow runs (Actions tab)
- Release pages (for release builds)

## Triggering Builds

Builds are triggered on:
- Push to `main` branch (creates release)
- Pull requests to `main` (builds only, no release)

Manual triggers:
```bash
# Trigger via GitHub CLI
gh workflow run build.yml
```

## Caching

The workflow caches:
- Cargo registry (~/.cargo/registry)
- Cargo git index (~/.cargo/git)
- Target build directory (target/)

This speeds up builds significantly after the first run.

## Testing Locally

To test cross-compilation locally:

```bash
# Add target
rustup target add x86_64-pc-windows-msvc

# Build for target
cargo build --locked --release --target x86_64-pc-windows-msvc

# Linux ARM64 (requires cross-compilation setup)
rustup target add aarch64-unknown-linux-gnu
sudo apt-get install gcc-aarch64-linux-gnu
cargo build --locked --release --target aarch64-unknown-linux-gnu
```

## Troubleshooting

### espeak-ng compilation failures
espeak-ng is compiled from source and requires:
- C compiler (gcc/clang)
- autoconf, automake, libtool (Linux)

### ObjC bridge compilation failures (macOS)
The macos_bridge.m requires:
- Xcode or Command Line Tools installed
- AVFoundation framework available

### Windows cross-compilation
Windows ARM64 builds work on windows-latest runners without extra setup because the MSVC toolchain includes ARM64 support.

### ARM64 cross-compilation
ARM64 builds for non-native platforms (Linux ARM64 on x64) require cross-compilation toolchains and proper linker configuration (see workflow env vars).
