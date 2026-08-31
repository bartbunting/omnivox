# GitHub Actions Workflow

[`build.yml`](build.yml) is the authoritative generic and Piper release matrix.
[`piper-native.yml`](piper-native.yml) is a manual, non-publishing validation
workflow for the optional Piper companion. User-facing artifact and
installation details are in [../DEPLOYMENT.md](../DEPLOYMENT.md).

## Jobs

### `format`

Runs on `ubuntu-latest`, installs Rust/rustfmt 1.97.1, and executes:

```sh
cargo fmt --all -- --check
python3 tools/check_markdown_links.py
```

The documentation check resolves repository-local links in every tracked
Markdown file. It does not make network requests for external URLs.

### `build`

Builds locked release binaries for five targets:

| Matrix name | Runner | Target |
|---|---|---|
| `linux-x64` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` |
| `macos-arm64` | `macos-15` | `aarch64-apple-darwin` |
| `macos-x64` | `macos-15-intel` | `x86_64-apple-darwin` |
| `windows-x64` | `windows-latest` | `x86_64-pc-windows-msvc` |
| `windows-arm64` | `windows-11-arm` | `aarch64-pc-windows-msvc` |

The Linux x64 artifact establishes Ubuntu 24.04 as its glibc and C++ runtime
build baseline. macOS and Windows use a native runner for each architecture.
Every artifact contains the main executable, `elisp/omnivox-voices.el`, the
matching generated `espeak-ng-data`, `LICENSE`, `LICENSING.md`, and third-party
notices. The build wrapper derives that data from the exact `espeak-rs-sys`
output reported by Cargo.

The main executable enables optional Piper helper discovery on every target.
This does not link libpiper into the server or add the companion to a generic
artifact; the tag-only and manual Piper jobs own that native payload.

The workflow verifies the data, project licensing files, and third-party
notices in all five artifacts. It also runs eSpeak voice discovery from the
packaged data on every native build runner and native WAV synthesis on both
macOS build runners.

### `test`

Runs `cargo test --locked` and
`cargo clippy --locked --all-targets -- -D warnings` on:

- Linux x64;
- macOS ARM64;
- macOS x64;
- Windows x64; and
- Windows ARM64.

The Linux job exercises the Rust tests and Clippy, but does not claim
audio-device playback coverage or compatibility with distributions older than
the Ubuntu 24.04 build baseline.

### `build_piper_release`

Tag builds add native Piper jobs for Linux x64, Windows x64, macOS ARM64, and
macOS x64. Each job performs the same locked-input, native Clippy, relocation,
25-synthesis, cancellation, real-synthesis, and fallback checks as the manual
workflow. The CI model remains outside every uploaded artifact.

### `package_piper_source_release`

The tag-only source job creates and verifies the platform-neutral
`omnivox-VERSION-piper-source.tar.gz`. It contains the recorded Git tree,
versioned Cargo sources, and seven unique locked native source/build-input
archives used by the four companions.

### `package_release`

Runs only for refs beginning `refs/tags/v` and depends on successful format,
generic build/test, four Piper build, and Piper source jobs. It rejects a tag
that does not match the compiled Linux binary version, restores Unix executable
modes after the Actions artifact round-trip, creates five generic archives,
adds four companions and the source archive, and writes one exhaustive SHA-256
file.

### `create_draft_release`

Uploads the packaged archives and checksums to a draft GitHub release. The
release is not public at this stage.

### `verify_release`

Each native runner downloads its exact archive plus `sha256sums.txt` from the
draft release. `tools/verify_release.py` verifies the checksum, safely extracts
the archive into a path containing spaces, checks the payload and executable
architecture, and runs from an unrelated working directory without runtime
path overrides.

Linux x64 executes eSpeak voice discovery and WAV synthesis. Windows x64 and
ARM64 execute both eSpeak and WinRT voice discovery and WAV synthesis. macOS
ARM64 and x64 execute eSpeak and AVSpeechSynthesizer.

### `verify_piper_release` and `verify_piper_source_release`

Each Piper runner downloads its exact companion, matching generic archive, and
unified checksum file from the draft. It safely extracts both, verifies native
layout and checksums, and performs real Piper synthesis plus
missing/corrupt-model fallback. The source verifier downloads the exact large
source asset and repeats Git-tree, manifest, input-lock, model-exclusion, and
offline Cargo checks from a checkout of the release tag.

### `publish_release`

Publishes the draft only after every generic, Piper, and source verification
passes. Any failure leaves the release as a draft for inspection.

The release does not package voice models, Eloquence or DECtalk helpers,
proprietary DLLs, or proprietary dictionaries.

### Piper native validation

The manual `piper-native.yml` workflow builds and atomically stages the
optional companion on Ubuntu 24.04 x64, Windows 2025 x64, macOS 15 ARM64, and
macOS 15 Intel. Each job prepares the platform's checksum-locked native
inputs, builds on the matching architecture, applies native layout and binary
checks, rechecks the input cache without network access, creates the native
`.tar.gz` or `.zip`, verifies it after safe extraction into a relocated path,
downloads and verifies the locked CI-only model, and runs real synthesis
through a Piper-enabled server. A persistent helper-session test performs 25
syntheses with varied supported settings, validates protocol/audio framing,
cancels one long in-flight synthesis without accepting stale output, confirms
the helper remains responsive, records observable memory, and requests clean
shutdown. The job then uploads the verified companion candidate plus its
checksum for inspection.

This provisional workflow does not publish release assets or a voice model.
The model remains outside the staged directory, companion archive, and upload
patterns. Its lock records the model card's public-domain LibriVox and
trained-from-scratch declarations and approves the exact revision only for CI
acceptance; passing this engineering test does not approve the model for
redistribution. This workflow remains useful for validating Piper without a
release tag.

## Triggers

```text
push to main       format + build + test
pull request main  format + build + test
v* tag             generic/Piper/source + draft + verify + publish
manual main ref    format + build + test
manual v* tag ref  format + build + test + package + draft + verify + publish
manual Piper flow  native companion package and verification; never publishes
```

`workflow_dispatch` is a fallback for manually running the same workflow. Run
it against `main` for the ordinary CI gates or against a `v*` tag to exercise
the complete release path. If a draft was created but verification could not
finish, dispatch against `main` with `draft_version` set to the version without
its leading `v` (for example, `1.5.0`). The workflow skips the build matrix,
downloads that existing draft's generic, Piper, and source assets, reruns every
native verification, and publishes only if every verifier passes.

## Caching

Build and test jobs cache the Cargo registry and target directory using keys
that include the target/matrix name and `Cargo.lock` hash. The workflow does
not cache a separate Cargo Git directory.

## Maintenance rules

- Keep the Rust action version synchronized with `rust-toolchain.toml`.
- Keep this summary and `.github/DEPLOYMENT.md` synchronized with the literal
  matrices and release archive loop in `build.yml`.
- Add a platform to documentation only after the workflow actually builds and,
  where claimed, tests it.
- Preserve `--locked` in build, test, and Clippy commands.
- Treat runner/toolchain changes and new release payloads as explicit release
  contract changes.
