# Deployment Guide

## GitHub Actions CI/CD

This repository is configured with automated builds for 6 platforms via GitHub Actions.

## What Gets Built

On every push to `main`, the following binaries are automatically built:

| Platform | Architecture | Binary |
|----------|-------------|--------|
| Windows | x64 | omnivox.exe |
| Windows | ARM64 | omnivox.exe |
| Linux | x64 | omnivox |
| Linux | ARM64 | omnivox |
| macOS | x64 (Intel) | omnivox |
| macOS | ARM64 (Apple Silicon) | omnivox |

## How It Works

1. **On push to main**: GitHub Actions triggers the workflow
2. **Builds run in parallel**: All 6 platform builds run concurrently
3. **Tests run**: Code is tested on Linux, Windows, and macOS
4. **Artifacts are uploaded**: Each binary is uploaded as a workflow artifact
5. **Release is created**: A new release is automatically created with all binaries

## Accessing Binaries

### From GitHub Releases

1. Go to https://github.com/robertmeta/omnivox/releases
2. Find the latest automated build (tagged `build-YYYYMMDD-HHMMSS`)
3. Download the archive for your platform:
   - Windows: `omnivox-windows-{x64,arm64}.zip`
   - Linux: `omnivox-linux-{x64,arm64}.tar.gz`
   - macOS: `omnivox-macos-{x64,arm64}.tar.gz`

### From Workflow Runs

1. Go to https://github.com/robertmeta/omnivox/actions
2. Click on a workflow run
3. Scroll to "Artifacts" section
4. Download the artifact for your platform

## Installation

### Windows
```powershell
# Extract zip
Expand-Archive omnivox-windows-x64.zip -DestinationPath C:\Program Files\omnivox

# Add to PATH or use full path
C:\Program Files\omnivox\omnivox.exe
```

### Linux
```bash
# Extract tarball
tar xzf omnivox-linux-x64.tar.gz

# Install to /usr/local/bin
sudo install -m 755 omnivox /usr/local/bin/omnivox

# Or install to user directory
mkdir -p ~/.local/bin
install -m 755 omnivox ~/.local/bin/omnivox
```

### macOS
```bash
# Extract tarball
tar xzf omnivox-macos-arm64.tar.gz

# Install to /usr/local/bin
sudo install -m 755 omnivox /usr/local/bin/omnivox

# Or install via Homebrew (if available)
brew install robertmeta/emacspeak/omnivox
```

## Platform-Specific Notes

### Windows
- Uses WinRT SpeechSynthesizer for native TTS
- Requires Windows 10 or later
- No additional dependencies required

### Linux
- Uses espeak-ng (statically compiled into the binary)
- No external dependencies required
- Works on any Linux distro with glibc 2.31+ (Ubuntu 20.04+)

### macOS
- Uses AVSpeechSynthesizer for native TTS
- Requires macOS 10.15 (Catalina) or later
- No additional dependencies required

### Cross-Platform Fallback

All binaries include espeak-ng compiled in. To use espeak-ng instead of the native TTS:

```bash
OMNIVOX_ENGINE=espeak omnivox
```

## Testing Builds Locally

To test the cross-compilation setup locally:

### Windows (on Windows)
```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --locked --release --target x86_64-pc-windows-msvc
```

### Linux (on Linux)
```bash
# Native build
cargo build --locked --release --target x86_64-unknown-linux-gnu

# ARM64 cross-compilation
sudo apt-get install gcc-aarch64-linux-gnu
rustup target add aarch64-unknown-linux-gnu
cargo build --locked --release --target aarch64-unknown-linux-gnu
```

### macOS (on macOS)
```bash
# Intel build
rustup target add x86_64-apple-darwin
cargo build --locked --release --target x86_64-apple-darwin

# Apple Silicon build
rustup target add aarch64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin
```

## Triggering Manual Builds

To manually trigger a build without pushing to main:

```bash
gh workflow run build.yml
```

Or via the GitHub web interface:
1. Go to Actions tab
2. Select "Build" workflow
3. Click "Run workflow"
4. Select branch and click "Run workflow"

## Build Matrix

The workflow uses a matrix strategy to build all platforms in parallel:

```yaml
matrix:
  include:
    - os: windows-latest
      target: x86_64-pc-windows-msvc
    - os: windows-latest
      target: aarch64-pc-windows-msvc
    - os: ubuntu-latest
      target: x86_64-unknown-linux-gnu
    - os: ubuntu-latest
      target: aarch64-unknown-linux-gnu
    - os: macos-latest
      target: x86_64-apple-darwin
    - os: macos-latest
      target: aarch64-apple-darwin
```

Each build job:
1. Installs Rust with the target triple
2. Installs platform-specific build tools
3. Builds with `cargo build --locked --release --target <triple>` using the
   exact Rust release in `rust-toolchain.toml`
4. Uploads the binary as an artifact

## Caching

To speed up builds, the workflow caches:
- Cargo registry (`~/.cargo/registry`)
- Cargo git index (`~/.cargo/git`)
- Build artifacts (`target/`)

First builds take 10-15 minutes per platform. Subsequent builds with caching take 2-5 minutes.

## Troubleshooting

### Build failures on Linux ARM64
Ensure gcc-aarch64-linux-gnu is installed and linker is configured:
```bash
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
```

### macOS ObjC bridge compilation errors
Ensure Xcode Command Line Tools are installed:
```bash
xcode-select --install
```

### espeak-ng compilation errors
espeak-ng requires C compiler and build tools:
- Linux: `build-essential autoconf automake libtool`
- macOS: Xcode Command Line Tools
- Windows: Visual Studio build tools (pre-installed on GitHub runners)

## Release Tagging Strategy

Releases are tagged automatically with timestamps:
- Format: `build-YYYYMMDD-HHMMSS`
- Example: `build-20260209-143022`

This allows tracking builds by date/time and provides chronological ordering.

## Future Enhancements

Potential improvements to CI/CD:

1. **Semantic versioning**: Use git tags for release versions
2. **Docker images**: Build container images for server deployment
3. **Homebrew automation**: Auto-update homebrew tap on release
4. **Binary signing**: Code-sign macOS/Windows binaries
5. **Cross-platform testing**: Test binaries on target platforms before release
6. **Performance benchmarks**: Run performance tests in CI
