# GitHub Actions Workflow

[`build.yml`](build.yml) is the only checked-in workflow and is the
authoritative build matrix. User-facing artifact and installation details are
in [../DEPLOYMENT.md](../DEPLOYMENT.md).

## Jobs

### `format`

Runs on `ubuntu-latest`, installs Rust/rustfmt 1.97.1, and executes:

```sh
cargo fmt --all -- --check
```

### `build`

Builds locked release binaries for five targets:

| Matrix name | Runner | Target |
|---|---|---|
| `linux-x64` | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` |
| `macos-arm64` | `macos-15` | `aarch64-apple-darwin` |
| `macos-x64` | `macos-15` | `x86_64-apple-darwin` |
| `windows-x64` | `windows-latest` | `x86_64-pc-windows-msvc` |
| `windows-arm64` | `windows-11-arm` | `aarch64-pc-windows-msvc` |

The Linux x64 artifact establishes Ubuntu 24.04 as its glibc and C++ runtime
build baseline. The macOS ARM runner cross-compiles the Intel target. Windows
uses a native runner for each architecture because the WinRT build requires it.
Every artifact contains the main executable, `elisp/omnivox-voices.el`, the
matching generated `espeak-ng-data`, and third-party notices. The build wrapper
derives that data from the exact `espeak-rs-sys` output reported by Cargo.

The workflow verifies the data and license files in all five artifacts. It also
runs eSpeak voice discovery from the packaged data on Linux x64, macOS ARM64,
and both native Windows targets. The cross-compiled macOS Intel executable is
not run on the ARM build runner.

### `test`

Runs `cargo test --locked` and `cargo clippy --locked -- -D warnings` on:

- Linux x64;
- macOS ARM64;
- Windows x64; and
- Windows ARM64.

The Linux job exercises the Rust tests and Clippy, but does not claim
audio-device playback coverage or compatibility with distributions older than
the Ubuntu 24.04 build baseline.

### `package_release`

Runs only for refs beginning `refs/tags/v` and depends on successful format,
build, and test jobs. It rejects a tag that does not match the compiled Linux
binary version, restores Unix executable modes after the Actions artifact
round-trip, creates three `.tar.gz` and two `.zip` archives, and writes SHA-256
checksums.

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
ARM64 executes eSpeak and AVSpeechSynthesizer. The cross-compiled macOS x64
archive receives structural and architecture checks but is not executed.

### `publish_release`

Publishes the draft only after every release-verification matrix entry passes.
Any failure leaves the release as a draft for inspection.

The release does not package optional Piper, Eloquence, or DECtalk helpers,
models, proprietary DLLs, or proprietary dictionaries.

## Triggers

```text
push to main       format + build + test
pull request main  format + build + test
v* tag             format + build + test + package + draft + verify + publish
```

There is no `workflow_dispatch` trigger, so `gh workflow run build.yml` is not
a supported manual entry point unless the workflow is changed first.

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
