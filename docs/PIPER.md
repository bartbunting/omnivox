# Piper Companion Guide

Piper is an optional out-of-process engine. Beginning with v1.6.1, Omnivox
publishes companion archives for Linux x64, Windows x64, macOS Apple Silicon,
and macOS Intel. Releases through v1.5.1 do not contain a Piper companion, so
use v1.6.1 or later, or build the current source.

## Payload boundary

The main `omnivox` executable discovers Piper but does not link libpiper. The
separate companion owns libpiper, ONNX Runtime, and its matching generated
eSpeak data:

```text
installation directory/
├── omnivox or omnivox.exe
├── espeak-ng-data/             main eSpeak engine data
└── piper/
    ├── omnivox-piper-helper or omnivox-piper-helper.exe
    ├── libpiper and ONNX Runtime libraries for this platform
    └── espeak-ng-data/         private to the Piper helper
```

Keep the entire `piper/` directory together. Do not merge its eSpeak data with
the main executable's `espeak-ng-data/`. A voice model is a separate
user-supplied `.onnx` file with an adjacent `<model>.onnx.json` or
`<model>.json` configuration; it is not part of the companion.

## Build and verify

Install the platform build prerequisites from the repository
[README](../README.md), then run:

```sh
make build-piper
PIPER_MODEL=/absolute/path/to/voice.onnx make verify-piper
```

`make build-piper` produces a Piper-enabled `target/release/omnivox` (or
`omnivox.exe`) and the isolated `target/release/piper/` directory.
`make verify-piper` rebuilds a clean companion candidate, safely extracts it
into a relocated path, checks its architecture and dynamic libraries, and—when
`PIPER_MODEL` is set—performs real synthesis.

To copy the current source build to the normal Cargo binary directory, stop
any running Omnivox process first and run:

```sh
make install-piper
```

Set `OMNIVOX_INSTALL_BIN=/absolute/destination` to choose another installation
directory. This command installs the main executable and copies the main
eSpeak payload, notices, project licensing files, and complete `piper/`
companion. It does not install a model.

## Install a companion archive

A published archive is named for its platform:

- `omnivox-VERSION-piper-linux-x64.tar.gz`;
- `omnivox-VERSION-piper-windows-x64.zip`;
- `omnivox-VERSION-piper-macos-arm64.tar.gz`; or
- `omnivox-VERSION-piper-macos-x64.tar.gz`.

Verify it against the accompanying `piper-sha256sums.txt`. Extract it into the
directory that already contains the matching Piper-enabled `omnivox`
executable. The archive has one top-level `piper/` directory, so extraction
creates the layout shown above. Do not combine a companion with a main
executable from another version or source commit; `SOURCE-PROVENANCE.json`
inside the companion records its exact Omnivox source.

The manual Piper workflow retains its builds as non-publishing engineering
artifacts. Beginning with v1.6.1, the tag workflow separately builds release
archives, downloads them back from a draft, and requires native real synthesis
before publication.

The platform-neutral `omnivox-VERSION-piper-source.tar.gz` archive contains
the committed Omnivox/libpiper source, locked Cargo sources, and the exact
source and binary inputs used across all four companions. Build it with
`make package-piper-source` and verify it with
`python3 tools/verify_piper_source.py`. It is intended to be published beside
the four native companions, not installed as a runtime payload.

## Configure and test a model

Review the model's `MODEL_CARD`, then keep its model and configuration
together. Configure server mode with an absolute path:

```sh
export OMNIVOX_PIPER_MODEL=/absolute/path/to/voice.onnx
omnivox --engine piper --list-voices
omnivox --engine piper --dump-wav "piper:VOICE_ID" piper-test.wav "Piper is configured."
```

In PowerShell:

```powershell
$env:OMNIVOX_PIPER_MODEL = "C:\absolute\path\to\voice.onnx"
omnivox.exe --engine piper --list-voices
omnivox.exe --engine piper --dump-wav "piper:VOICE_ID" piper-test.wav "Piper is configured."
```

Replace `piper:VOICE_ID` with the exact `piper:...` identifier reported by
`--list-voices`.

`--piper-model` can override the model for server and diagnostic actions,
including `--check` and `--dump-wav`. `OMNIVOX_PIPER_MODEL` remains the
persistent alternative. See
[ENV-VARS.md](ENV-VARS.md) for helper and eSpeak-data overrides.

If the helper, model, configuration, or native runtime is unavailable, Piper
is not registered in server mode and normal platform/eSpeak routes remain. A
single-action `--engine piper` diagnostic fails if the exact Piper helper,
model, configuration, or runtime is unavailable, so a successful command is
evidence that Piper produced the result.

## Upgrade or remove

Stop Omnivox before replacing a companion, especially on Windows where loaded
DLLs cannot be replaced reliably. Upgrade the main executable and the complete
matching `piper/` directory together, then repeat voice discovery and WAV
synthesis before restarting Emacspeak or Emacsvox.

To disable Piper without deleting files, unset `OMNIVOX_PIPER_MODEL` and remove
any `--piper-model` option. To remove it, stop Omnivox and remove only the
installation's `piper/` directory. Preserve the main `espeak-ng-data/` and
`third-party-licenses/` directories; they belong to the generic Omnivox
payload. An unavailable Piper companion does not disable the platform-native
or eSpeak engines.

See [LICENSING.md](LICENSING.md) for the source, native-library, and model
licensing boundaries. The engineering CI model is explicitly not included in
companion artifacts and does not constitute a general model endorsement.
