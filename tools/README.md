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
synthesis calls. It validates negotiation, descriptor identity, realized voice,
audio sequence and frame totals, word/sentence marker bounds, periodic pings,
and clean shutdown.

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
