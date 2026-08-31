# Developer Tools

## Documentation links

`check_markdown_links.py` resolves repository-local links in every tracked
Markdown file and rejects missing targets, unsupported URI schemes, and links
that leave the repository. It does not make network requests for external
URLs. Run it directly or through:

```sh
make docs-check
```

## Build and runtime staging

`build.py` is the supported wrapper for distributable Cargo builds. It keeps
Cargo's locked dependency resolution, reads the exact `espeak-rs-sys` output
from Cargo's JSON build messages, and stages `espeak-ng-data` plus applicable
project and third-party licensing files beside the executable. The Makefile and
release workflow invoke it; additional Cargo build arguments pass through
unchanged:

```sh
python3 tools/build.py --release
python3 tools/build.py --release --target aarch64-apple-darwin
```

The wrapper fails rather than choose between non-identical eSpeak data outputs.

`build_piper.py` builds the optional helper in relocatable mode, selects the
native Linux x64, Windows x64, or macOS ARM64/x64 library layout, and
atomically stages the companion as `piper/` beside the Cargo profile output.
The directory contains the helper, only the required native libraries, its
separately generated eSpeak data, project and third-party licensing files,
source/input provenance, and a SHA-256 manifest:

```sh
python3 tools/build_piper.py --release
```

Linux staging validates `$ORIGIN` lookup and resolved libraries; macOS staging
validates thin Mach-O architecture and `@loader_path` lookup; Windows staging
validates x64 PE identities. Native runner synthesis remains the final dynamic
loading check on every platform. Before Cargo runs,
`prepare_piper_inputs.py` downloads the target's checked-in eSpeak NG, Sonic,
and ONNX Runtime archives, verifies their SHA-256 digests, safely extracts
them, and records the extracted-tree digests. It detects Linux x64, Windows
x64, and macOS ARM64/x64 native targets; `--target` remains available for an
explicit cache. A prepared cache can be checked without network access:

```sh
python3 tools/prepare_piper_inputs.py \
  --target x86_64-unknown-linux-gnu --check
```

`prepare_piper_test_model.py` separately downloads the English model used by
upstream libpiper tests at a locked repository revision, verifies the model,
configuration, and `MODEL_CARD` sizes and SHA-256 digests, and records the
prepared state under `target/piper-test-model/`:

```sh
make prepare-piper-test-model
python3 tools/prepare_piper_test_model.py --check
```

This model is for native acceptance only. Its checked-in lock records the
model card's LibriVox, public-domain, and trained-from-scratch declarations,
approves that exact revision for CI-only use, and requires that the model stay
out of release artifacts.

Create and verify the deterministic companion candidate for the current native
platform with:

```sh
make package-piper
make verify-piper
```

The result is one of the following, with its outer digest written to
`target/release/piper-sha256sums.txt`:

| Native platform | Companion candidate |
| --- | --- |
| Linux x64 | `omnivox-VERSION-piper-linux-x64.tar.gz` |
| macOS ARM64 | `omnivox-VERSION-piper-macos-arm64.tar.gz` |
| macOS x64 | `omnivox-VERSION-piper-macos-x64.tar.gz` |
| Windows x64 | `omnivox-VERSION-piper-windows-x64.zip` |

Each archive extracts as one `piper/` directory, ready to place beside the
matching generic `omnivox` executable. To add real synthesis through the
relocated main binary, supply a licence-reviewed model and adjacent
configuration:

```sh
PIPER_MODEL=/path/to/voice.onnx make verify-piper
```

Create and verify the platform-neutral corresponding-source artifact with:

```sh
make package-piper-source
python3 tools/verify_piper_source.py
```

`package_piper_source.py` archives the exact committed Omnivox and vendored
libpiper tree, every Cargo registry source selected by `Cargo.lock`, the
checksum-locked eSpeak NG and Sonic sources, all four ONNX Runtime binary build
inputs, and the exact ONNX Runtime source revision. The deterministic result is
`target/release/omnivox-VERSION-piper-source.tar.gz`. Its exhaustive manifest
records every file's mode, size, and SHA-256 digest. The verifier compares the
Omnivox tree to its recorded Git commit, rechecks every input against the
archived locks, and resolves the Cargo graph offline from the included vendor
directory. CI voice-model payloads are excluded.

`verify_piper_release.py` checks the outer and exhaustive inner checksums,
exact payload and notice sets, clean source provenance, native binary
architecture, platform library lookup, and optional model-backed voice
discovery and WAV output. Linux uses ELF/RUNPATH and `ldd` checks, macOS uses
Mach-O, `@loader_path`, and `otool` checks, and Windows validates the PE import
chain from the helper through `piper.dll` to ONNX Runtime. When a model is
supplied, it also confirms that missing and corrupt Piper models preserve the
eSpeak fallback. Model-backed archive acceptance passes on all four initial
native runners. The tag workflow packages and re-verifies the four companions
and source artifact. Release code remains unsigned, and no Piper archive is
published yet.

## Release archive verification

`verify_release.py` checks an exact `.tar.gz` or `.zip` release asset against
`sha256sums.txt`. It safely extracts into a relocated path, validates the
published payload and binary architecture, and can exercise real headless
engine synthesis through `--dump-wav` without requiring an audio device.

For example:

```sh
python3 tools/verify_release.py \
  --archive omnivox-1.4.1-linux-x64.tar.gz \
  --checksums sha256sums.txt \
  --version 1.4.1 \
  --platform linux \
  --arch x86_64 \
  --engines espeak
```

The tag workflow uploads draft release assets first, downloads them on the
native platform runners, and publishes only after this verification passes.
Every current release target performs native synthesis verification. An empty
engine list remains available for an intentional structural and architecture
check that does not exercise synthesis.

## Failure diagnostics

`collect_diagnostics.sh` creates a bounded archive containing recent Omnivox
session logs, build/runtime identity, process inventory, and relevant Windows
events. It does not include Windows memory dumps. See
[`docs/DIAGNOSTICS.md`](../docs/DIAGNOSTICS.md) for the failure workflow and the
opt-in `configure_windows_crash_dumps.ps1` helper.

## Windows helper session stress

`stress_helper.py` keeps one protocol-v4 helper process alive across repeated
synthesis calls. It validates negotiation, descriptor identity, realized
voice, audio sequence and frame totals, all marker kinds advertised by the
engine, periodic pings, and clean shutdown. With `--cancel-probe`, it also
cancels one long in-flight synthesis, rejects stale output after the
acknowledgement, and checks that the helper remains responsive. It records
process memory on systems with a compatible `/proc` entry. The native Piper
workflow uses both paths on every initial platform.

From WSL, after building the Emacsvox helpers:

```sh
python3 tools/stress_helper.py \
  ~/src/emacsvox/servers/windows-eloquence/bin/OmnivoxEloquenceHelper32.exe \
  --engine-id eloquence --iterations 100

python3 tools/stress_helper.py \
  ~/src/emacsvox/servers/windows-dectalk/bin/OmnivoxDectalkHelper32.exe \
  --engine-id dectalk --iterations 100
```

Use `--voice-id` to select a non-default voice and repeat `--helper-arg` when a
helper needs an explicit native DLL argument. The RSS value available from WSL
belongs to its interop launcher; use native Windows process tooling for a
working-set growth measurement.

The in-process eSpeak counterpart is ignored during ordinary unit tests and
can be run explicitly:

```sh
cargo test --locked -p omnivox-tts stress_repeated_synthesis_session \
  -- --ignored --nocapture
```

## Audible feature smoke test

[test-all-features.sh](../test-all-features.sh) exercises legacy protocol
commands, the selected native and eSpeak startup engines, integer speech rates,
and left/right/both channel routing. It also exercises Piper when both
`PIPER_MODEL` and an adjacent or explicitly selected helper are available:

```sh
OMNIVOX_BIN=./target/release/omnivox ./test-all-features.sh
```

The script requires Bash and `timeout`. Its pass/fail result detects process
errors and timeouts; a listener must still verify the audible rate and channel
claims. This is a manual smoke test, not a CI latency or audio-quality gate.

## WAV comparison

`compare_wavs.py` compares two PCM16 or float32 WAV files after silence
trimming and reports RMS difference, correlation, SNR, and per-segment values.
`tts_reference.swift` captures a macOS AVSpeechSynthesizer reference WAV.
