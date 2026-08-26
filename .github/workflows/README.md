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

Builds locked release binaries for four targets:

| Matrix name | Runner | Target |
|---|---|---|
| `macos-arm64` | `macos-15` | `aarch64-apple-darwin` |
| `macos-x64` | `macos-15` | `x86_64-apple-darwin` |
| `windows-x64` | `windows-latest` | `x86_64-pc-windows-msvc` |
| `windows-arm64` | `windows-11-arm` | `aarch64-pc-windows-msvc` |

The macOS ARM runner cross-compiles the Intel target. Windows uses a native
runner for each architecture because the WinRT build requires it. Every
artifact contains the main executable, `elisp/omnivox-voices.el`, the matching
generated `espeak-ng-data`, and third-party notices. The build wrapper derives
that data from the exact `espeak-rs-sys` output reported by Cargo.

The workflow verifies the data and license files in all four artifacts. It also
runs eSpeak voice discovery from the packaged data on macOS ARM64 and both
native Windows targets. The cross-compiled macOS Intel executable is not run on
the ARM build runner.

There is no Linux build in this matrix.

### `test`

Runs `cargo test --locked` and `cargo clippy --locked -- -D warnings` on:

- macOS ARM64;
- Windows x64; and
- Windows ARM64.

There is no Linux runtime-test job. Formatting on Ubuntu must not be described
as Linux test coverage.

### `release`

Runs only for refs beginning `refs/tags/v` and depends on successful format,
build, and test jobs. It downloads the four artifacts, creates two `.tar.gz`
and two `.zip` archives, writes SHA-256 checksums, and publishes them through a
GitHub release.

The release does not package optional Piper, Eloquence, or DECtalk helpers,
models, proprietary DLLs, or proprietary dictionaries.

## Triggers

```text
push to main       format + build + test
pull request main  format + build + test
v* tag             format + build + test + release
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
