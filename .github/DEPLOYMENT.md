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

Each archive contains the main binary, portable RHVoice helper,
`omnivox-voices.el`, the matching generated `espeak-ng-data`, `LICENSE`,
`LICENSING.md`, and `third-party-licenses`. It does not contain the RHVoice
runtime or voice data. A successful current tag workflow also publishes these
optional Piper assets:

| Platform | Companion archive |
|---|---|
| Linux x64 | `omnivox-VERSION-piper-linux-x64.tar.gz` |
| macOS Apple Silicon | `omnivox-VERSION-piper-macos-arm64.tar.gz` |
| macOS Intel | `omnivox-VERSION-piper-macos-x64.tar.gz` |
| Windows x64 | `omnivox-VERSION-piper-windows-x64.zip` |
| All four | `omnivox-VERSION-piper-source.tar.gz` |

The same workflow publishes these SLT-only Flite companions:

| Platform | Companion archive |
|---|---|
| Linux x64 | `omnivox-VERSION-flite-linux-x64.tar.gz` |
| Linux ARM64 | `omnivox-VERSION-flite-linux-arm64.tar.gz` |
| macOS Apple Silicon | `omnivox-VERSION-flite-macos-arm64.tar.gz` |
| macOS Intel | `omnivox-VERSION-flite-macos-x64.tar.gz` |
| Windows x64 | `omnivox-VERSION-flite-windows-x64.zip` |
| Windows ARM64 | `omnivox-VERSION-flite-windows-arm64.zip` |
| All six | `omnivox-VERSION-flite-source.tar.gz` |

It also publishes these self-contained RuTTS companions with built-in male and
female Russian voices:

| Platform | Companion archive |
|---|---|
| Linux x64 | `omnivox-VERSION-rutts-linux-x64.tar.gz` |
| Linux ARM64 | `omnivox-VERSION-rutts-linux-arm64.tar.gz` |
| macOS Apple Silicon | `omnivox-VERSION-rutts-macos-arm64.tar.gz` |
| macOS Intel | `omnivox-VERSION-rutts-macos-x64.tar.gz` |
| Windows x64 | `omnivox-VERSION-rutts-windows-x64.zip` |
| Windows ARM64 | `omnivox-VERSION-rutts-windows-arm64.zip` |
| All six | `omnivox-VERSION-rutts-source.tar.gz` |

Releases also publish one `sha256sums.txt` covering every generic, companion,
and source archive. The workflow does **not** publish Linux ARM64 or Windows
ARM64 Piper companions, voice models, RuLex, proprietary-engine helpers, or
proprietary runtimes.

Published release `v1.4.1` predates the root `LICENSE` and `LICENSING.md`
archive entries. The verifier accepts that one historical layout; all later
archives must contain both files.

The Linux x64 archive is built and tested on Ubuntu 24.04 and requires
compatible glibc, libstdc++, libgcc, and ALSA runtime libraries. Compatibility
with older distributions is not claimed. Linux ARM64 may build from source, but
the project does not currently claim CI or runtime support for that target.

## Workflow triggers

- A push or pull request to `main` runs formatting, builds, and tests.
- A tag matching `v*` runs the same gates, uploads a draft GitHub release,
  verifies the downloaded archives on their native runners, and publishes only
  after every required job succeeds.
- A manual dispatch against `main` runs the ordinary CI gates; a dispatch
  against a `v*` tag runs the complete release path.
- A manual dispatch against `main` with `draft_version` set resumes verification
  of an existing draft and publishes it only after all native verifiers pass.
- An ordinary push to `main` does not create a timestamped release.

The release version and archive prefix come from the tag name with its leading
`v` removed for filenames. For example, tag `v1.5.0` produces archives prefixed
`omnivox-1.5.0-`.

## What CI validates

- Formatting on an Ubuntu runner.
- Release builds for Linux x64 and both listed macOS and Windows architectures.
- Tests and Clippy on Linux x64, macOS ARM64, macOS x64, Windows x64, and
  Windows ARM64.
- Presence of the project licensing files and packaged eSpeak data and notices
  on every artifact, plus packaged eSpeak voice discovery on every native build
  target.
- Native AVSpeechSynthesizer WAV synthesis during both macOS build jobs.
- Tag-to-binary version agreement, release checksums, safe extraction, root
  payload layout, executable modes and architectures, and adjacent eSpeak data
  discovery from a relocated directory without path overrides.
- Non-empty canonical WAV synthesis through eSpeak on Linux x64; through eSpeak
  and WinRT on Windows x64 and ARM64; and through eSpeak and
  AVSpeechSynthesizer on macOS ARM64 and x64.
- Native Flite companion builds, archives, relocation, repeated SLT synthesis,
  ACSS reporting, cancellation, and shutdown on Linux x64/ARM64, macOS
  Intel/Apple Silicon, and Windows x64/ARM64, plus the exact deterministic
  source artifact and offline source preparation.
- Native RuTTS companion builds, archives, relocation, repeated male/female
  synthesis, ACSS reporting, cancellation, and shutdown on Linux x64/ARM64,
  macOS Intel/Apple Silicon, and Windows x64/ARM64, plus the exact deterministic
  source artifact, RuLex-exclusion check, and offline source preparation.
- Native Piper companion staging, linkage, relocation, persistent synthesis,
  cancellation, missing/corrupt-model fallback, and exact draft-asset Piper
  synthesis on Linux x64, Windows x64, and macOS ARM64/x64.
- The exact deterministic Piper source/build-input artifact, including its Git
  tree, exhaustive manifest, locked native inputs, CI-model exclusion, and
  offline Cargo graph.
- Locked dependency resolution with Rust 1.97.1, matching
  `rust-toolchain.toml`.

The publishing workflow does not exercise real Eloquence or DECtalk runtimes,
physical audible onset or audio-device playback, or Emacsvox's content-addressed
Windows staging contract. The separate manual Piper workflow remains available
for non-publishing engineering validation. Any failed generic, Flite, RuTTS,
Piper, source, or draft-asset verification gate leaves the GitHub release
unpublished.

## Installing an archive

Download one archive and `sha256sums.txt` from the same GitHub release. Verify
the archive before extracting it. Replace `VERSION` in these examples with the
release version.

On Linux:

```sh
archive=omnivox-VERSION-linux-x64.tar.gz
grep -F "  $archive" sha256sums.txt | sha256sum --check
tar -xzf "$archive"
```

On macOS, select the archive matching Apple Silicon or Intel:

```sh
archive=omnivox-VERSION-macos-arm64.tar.gz # or ...-macos-x64.tar.gz
grep -F "  $archive" sha256sums.txt | shasum -a 256 --check
tar -xzf "$archive"
```

On Windows PowerShell, select the archive matching x64 or ARM64:

```powershell
$archive = "omnivox-VERSION-windows-x64.zip" # or ...-windows-arm64.zip
$checksumLine = (Select-String -Path sha256sums.txt -SimpleMatch "  $archive").Line
if (-not $checksumLine) { throw "$archive is absent from sha256sums.txt" }
$expected = ($checksumLine -split '\s+')[0]
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw "SHA-256 mismatch for $archive" }
Expand-Archive -LiteralPath $archive -DestinationPath omnivox-release
Set-Location omnivox-release
```

An empty checksum lookup is an error: confirm that the archive and checksum
file came from the same release. After extraction, keep the binary,
`espeak-ng-data`, `third-party-licenses`, and `rhvoice` together. Keep the
matching adapter from the same release as well. `LICENSE` and `LICENSING.md`
document the source and combined-binary distribution terms; preserve them with
redistributed copies.

On Linux or macOS, place `omnivox` in a directory on `PATH` and make it
executable. Linux also requires compatible system C++, GCC, glibc, and ALSA
runtime libraries:

```sh
binary_directory="$HOME/.local/bin"
adapter_directory="$HOME/.emacs.d/lisp/omnivox"
mkdir -p "$binary_directory/espeak-ng-data" \
  "$binary_directory/third-party-licenses" \
  "$binary_directory/rhvoice" "$adapter_directory"
install -m 755 omnivox "$binary_directory/omnivox"
install -m 644 omnivox-voices.el "$adapter_directory/omnivox-voices.el"
cp -R espeak-ng-data/. "$binary_directory/espeak-ng-data/"
cp -R third-party-licenses/. "$binary_directory/third-party-licenses/"
cp -R rhvoice/. "$binary_directory/rhvoice/"
```

On Windows, copy the extracted payload together into the Emacspeak
speech-server directory or another configured executable location. Release
archives use WinRT as the native Windows engine; the adjacent packaged data
makes eSpeak available as a fallback without a separate installation:

```powershell
$destination = "$env:USERPROFILE\.emacspeak\servers"
$directories = @(
  $destination,
  "$destination\espeak-ng-data",
  "$destination\third-party-licenses",
  "$destination\rhvoice"
)
New-Item -ItemType Directory -Force $directories | Out-Null
Copy-Item -Force omnivox.exe, omnivox-voices.el $destination
Copy-Item -Recurse -Force espeak-ng-data\* "$destination\espeak-ng-data"
Copy-Item -Recurse -Force third-party-licenses\* "$destination\third-party-licenses"
Copy-Item -Recurse -Force rhvoice\* "$destination\rhvoice"
```

For a release that lists Piper, verify and extract the matching companion into
the generic executable's directory; its one top-level `piper/` directory keeps
the runtime isolated. Supply a separately reviewed voice model and follow the
[Piper companion guide](../docs/PIPER.md). Other optional helper engines still
require adjacent executables and user-supplied runtimes. For Flite, extract the
matching companion's `flite/` directory beside the generic executable; its
built-in SLT voice requires no additional runtime. See the
[Flite companion guide](../docs/FLITE.md). For RuTTS, extract the matching
companion's `rutts/` directory beside the generic executable; its built-in
male and female voices require no additional runtime. See the
[RuTTS companion guide](../docs/RUTTS.md). The generic `rhvoice/` helper still
requires a separately installed compatible runtime and voice; see the
[RHVoice guide](../docs/RHVOICE.md). The
[Windows helper guide](../windows-helpers/README.md#runtime-requirements-and-installation)
documents the complete Eloquence and DECtalk requirements, the durable DECtalk
binary, and the separately labelled newer-build path.

Release binaries are not code-signed or notarized. macOS Gatekeeper or Windows
SmartScreen may therefore warn or refuse the first launch. Verify the SHA-256
checksum first, do not disable platform protections globally, and use only an
OS-provided per-file exception that you understand. Build from reviewed source
when local policy requires signed software.

The repository adapter is for upstream Emacspeak. Follow [README.md](../README.md)
and [ENV-VARS.md](../docs/ENV-VARS.md) rather than mixing those `dtk-*` names with
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
Before pushing a final tag, use the supported build process to create and test a
commit-equivalent candidate payload on at least one physical Linux x64 and
Windows x64 system. Keep a short record of the commit, OS version, selected
voices, and audio device. This pre-tag check does not test the byte-identical
release archives: those archives exist only after the tag workflow packages
them and are then checked automatically, without listening, on native hosted
runners.

On Linux or macOS, point the audible feature test at the candidate binary:

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
matching Emacspeak or Emacsvox adapter from the same commit. These checks
remain advisory rather than a publication gate because process success and
generated PCM do not prove audible onset, channel placement, device selection,
or cancellation at the speaker.

## Current release gaps

- Code signing/notarization is not part of the workflow; see the installation
  warning above.
- Linux ARM64 artifacts and broad Linux distribution compatibility tests are
  absent for the generic server. Linux ARM64 Flite and RuTTS have native
  companion jobs.
- Optional helper/model packaging is separate from generic release archives.
- Performance/onset and real proprietary-engine smoke tests are not CI gates.
- Physical audible checks use commit-equivalent pre-tag builds; the workflow
  has no human approval gate for the exact tagged archives.
- Release artifact retention follows GitHub's configured/default policies; do
  not rely on a hard-coded retention duration in project documentation.
