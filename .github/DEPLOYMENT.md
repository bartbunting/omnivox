# Release and Deployment Guide

## Published artifacts

The checked-in GitHub Actions workflow publishes these release archives:

| Platform | Target | Archive |
|---|---|---|
| macOS Apple Silicon | `aarch64-apple-darwin` | `omnivox-VERSION-macos-arm64.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `omnivox-VERSION-macos-x64.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `omnivox-VERSION-windows-x64.zip` |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `omnivox-VERSION-windows-arm64.zip` |

Each archive contains the main binary and `omnivox-voices.el`. Releases also
publish `sha256sums.txt`. The workflow does **not** currently publish Linux
artifacts, optional Piper helpers/models, proprietary-engine helpers, or
proprietary runtimes.

Linux remains supported as a source build using eSpeak NG. Do not advertise a
downloadable Linux binary, ABI baseline, or ARM64 cross-build until the workflow
contains and validates that target.

## Workflow triggers

- A push or pull request to `main` runs formatting, builds, and tests.
- A tag matching `v*` runs the same gates and creates a GitHub release after
  every required job succeeds.
- An ordinary push to `main` does not create a timestamped release.

The release version and archive prefix come from the tag name with its leading
`v` removed for filenames. For example, tag `v1.3.0` produces archives prefixed
`omnivox-1.3.0-`.

## What CI validates

- Formatting on an Ubuntu runner.
- Release builds for both listed macOS and Windows architectures.
- Tests and Clippy on macOS ARM64, Windows x64, and Windows ARM64.
- Locked dependency resolution with Rust 1.97.1, matching
  `rust-toolchain.toml`.

The workflow does not currently execute Linux runtime tests. It also does not
exercise real Eloquence, DECtalk, or Piper runtimes, physical audible onset, or
Emacsvox's content-addressed Windows staging contract.

## Installing an archive

Verify the archive against `sha256sums.txt`, extract it, and keep the binary and
matching adapter from the same release.

On macOS, place `omnivox` in a directory on `PATH` and make it executable:

```sh
install -m 755 omnivox "$HOME/.local/bin/omnivox"
```

On Windows, copy `omnivox.exe` into the Emacspeak speech-server directory or
another configured executable location. Release archives use WinRT as the
native engine and include eSpeak NG fallback; optional helper engines require
separate adjacent executables and user-supplied runtimes.

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
cargo build --locked --release
```

Platform-specific claims still require the relevant host and runtime. A
cross-compile alone does not verify audio-device behavior, native voice
inventory, cancellation, or audible latency.

## Current release gaps

- Code signing/notarization is not part of the workflow.
- Linux artifacts and Linux runtime tests are absent.
- Optional helper/model packaging is separate from generic release archives.
- Performance/onset and real proprietary-engine smoke tests are not CI gates.
- Release artifact retention follows GitHub's configured/default policies; do
  not rely on a hard-coded retention duration in project documentation.
