# Developer Tools

## Build and runtime staging

`build.py` is the supported wrapper for distributable Cargo builds. It keeps
Cargo's locked dependency resolution, reads the exact `espeak-rs-sys` output
from Cargo's JSON build messages, and stages `espeak-ng-data` plus applicable
third-party notices beside the executable. The Makefile and release workflow
invoke it; additional Cargo build arguments pass through unchanged:

```sh
python3 tools/build.py --release
python3 tools/build.py --release --target aarch64-apple-darwin
```

The wrapper fails rather than choose between non-identical eSpeak data outputs.

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
