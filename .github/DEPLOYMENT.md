# Release and Deployment Guide

## Published artifacts

The checked-in GitHub Actions workflow publishes these release archives:

| Platform | Target | Archive |
|---|---|---|
| Linux x64 | `x86_64-unknown-linux-gnu` | `omnivox-VERSION-linux-x64.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `omnivox-VERSION-macos-arm64.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `omnivox-VERSION-macos-x64.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `omnivox-VERSION-windows-x64.zip` |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `omnivox-VERSION-windows-arm64.zip` |

Each archive contains the main binary, `omnivox-voices.el`, the matching
generated `espeak-ng-data`, and `third-party-licenses`. Releases also publish
`sha256sums.txt`. The workflow does **not** currently publish Linux ARM64
artifacts, optional Piper helpers/models, proprietary-engine helpers, or
proprietary runtimes.

The Linux x64 archive is built and tested on Ubuntu 24.04 and requires
compatible glibc, libstdc++, libgcc, and ALSA runtime libraries. Compatibility
with older distributions is not claimed. Linux ARM64 remains supported only as
a source build until the workflow builds and validates that target.

## Workflow triggers

- A push or pull request to `main` runs formatting, builds, and tests.
- A tag matching `v*` runs the same gates, uploads a draft GitHub release,
  verifies the downloaded archives on their native runners, and publishes only
  after every required job succeeds.
- A manual dispatch against `main` runs the ordinary CI gates; a dispatch
  against a `v*` tag runs the complete release path.
- An ordinary push to `main` does not create a timestamped release.

The release version and archive prefix come from the tag name with its leading
`v` removed for filenames. For example, tag `v1.4.0` produces archives prefixed
`omnivox-1.4.0-`.

## What CI validates

- Formatting on an Ubuntu runner.
- Release builds for Linux x64 and both listed macOS and Windows architectures.
- Tests and Clippy on Linux x64, macOS ARM64, macOS x64, Windows x64, and
  Windows ARM64.
- Presence of the packaged eSpeak data and license payload on every artifact,
  plus packaged eSpeak voice discovery on every native build target.
- Tag-to-binary version agreement, release checksums, safe extraction, root
  payload layout, executable modes and architectures, and adjacent eSpeak data
  discovery from a relocated directory without path overrides.
- Non-empty canonical WAV synthesis through eSpeak on Linux x64; through eSpeak
  and WinRT on Windows x64 and ARM64; and through eSpeak and
  AVSpeechSynthesizer on macOS ARM64 and x64.
- Locked dependency resolution with Rust 1.97.1, matching
  `rust-toolchain.toml`.

The workflow does not exercise real Eloquence, DECtalk, or Piper runtimes,
physical audible onset or audio-device playback, or Emacsvox's content-addressed
Windows staging contract. A failed archive verification leaves the GitHub
release in draft state.

## Installing an archive

Verify the archive against `sha256sums.txt`, extract it, and keep its binary,
`espeak-ng-data`, and `third-party-licenses` together. Keep the matching adapter
from the same release as well.

On Linux or macOS, place `omnivox` in a directory on `PATH` and make it
executable. Linux also requires compatible system C++, GCC, glibc, and ALSA
runtime libraries:

```sh
install -m 755 omnivox "$HOME/.local/bin/omnivox"
mkdir -p "$HOME/.local/bin/espeak-ng-data" "$HOME/.local/bin/third-party-licenses"
cp -R espeak-ng-data/. "$HOME/.local/bin/espeak-ng-data/"
cp -R third-party-licenses/. "$HOME/.local/bin/third-party-licenses/"
```

On Windows, copy the extracted payload together into the Emacspeak
speech-server directory or another configured executable location. Release
archives use WinRT as the native Windows engine; the adjacent packaged data
makes eSpeak available as a fallback without a separate installation. Optional
helper engines require separate adjacent executables and user-supplied
runtimes.

The repository adapter is for upstream Emacspeak. Follow [README.md](../README.md)
and [ENV-VARS.md](../ENV-VARS.md) rather than mixing those `dtk-*` names with
Emacsvox's bundled adapter.

## Emacsvox Windows deployment

Emacsvox does not consume this generic CI archive for its reproducible WSL to
Windows integration. The sibling Emacsvox repository owns a pinned Windows-GNU
build, 32-bit helper builds, runtime inputs, provenance, content-addressed
staging, and Windows-local runtime copy:

```sh
cd /path/to/emacsvox
make windows-omnivox       # clean reproducible release input
make windows-omnivox-dev   # active local changes with tracked-diff hashes
```

See `servers/omnivox-release/README.org` in that repository. Do not manually
copy over a running content-addressed runtime or weaken its clean-worktree
guard.

## Local release checks

Use the pinned toolchain and locked commands:

```sh
make fmt-check
cargo test --locked --workspace
make lint
make build
```

Platform-specific claims still require the relevant host and runtime. A
cross-compile alone does not verify audio-device behavior, native voice
inventory, cancellation, or audible latency.

For a local archive check, run `tools/verify_release.py` with the archive,
`sha256sums.txt`, version, platform, architecture, and applicable engine list.
The tool intentionally extracts into a path containing spaces and runs the
binary from another working directory without Omnivox or eSpeak path overrides.

## Physical release checks

Hosted runners prove synthesis into PCM, but not delivery through a real audio
device. The automated tag workflow does not wait for an audible sign-off.
Before pushing a final tag, extract a candidate payload from the same commit on
at least one physical Linux x64 and Windows x64 system and keep a short record
of the OS version, selected voices, and audio device.

On Linux or macOS, point the audible feature test at the extracted binary:

```sh
OMNIVOX_BIN=/path/to/extracted/omnivox ./test-all-features.sh
```

On Windows, set the same variable for the PowerShell smoke tests:

```powershell
$env:OMNIVOX_BIN = "C:\path\to\extracted\omnivox.exe"
.\test-speech.ps1
.\test-tones.ps1
.\test-audio-icons.ps1
.\test-tones-with-speech.ps1
```

Listen for native and eSpeak speech, tones, icons, left/right/both routing,
queue order, and prompt stop behavior. Also run `omnivox --check` and start the
matching Emacspeak or Emacsvox adapter from the extracted payload. These checks
remain manual because process success and generated PCM do not prove audible
onset, channel placement, device selection, or cancellation at the speaker.

## Current release gaps

- Code signing/notarization is not part of the workflow.
- Linux ARM64 artifacts and broad Linux distribution compatibility tests are
  absent.
- Optional helper/model packaging is separate from generic release archives.
- Performance/onset and real proprietary-engine smoke tests are not CI gates.
- Release artifact retention follows GitHub's configured/default policies; do
  not rely on a hard-coded retention duration in project documentation.
