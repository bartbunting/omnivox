# Flite Companion Guide

Omnivox ships Flite as a separate, source-built companion process. The
companion statically contains Flite v2.2 and exactly one voice,
`cmu_us_slt`. Users do not need a system Flite installation and Omnivox does
not download voices at run time.

The build is pinned to upstream commit
`e9e2e37c329dbe98bfeb27a1828ef9a71fa84f88`. Omnivox verifies the release
archive and complete extracted source tree before compiling it. The native
build uses the target C compiler selected by Cargo, so GCC, Clang, and MSVC
follow the same source list without Autotools.

## Install a release companion

Flite is intentionally separate from the generic Omnivox archive. Download the
Flite companion whose platform and architecture match the generic archive,
verify both with the release `sha256sums.txt`, and extract its one top-level
`flite/` directory beside `omnivox` or `omnivox.exe`:

```text
installation-directory/
  omnivox                 # omnivox.exe on Windows
  flite/
    omnivox-flite-helper  # .exe on Windows
    README.md
    SOURCE-PROVENANCE.json
    SHA256SUMS
    third-party-licenses/
```

The main server discovers that layout automatically. An explicit helper path
can instead be set with `OMNIVOX_FLITE_HELPER`.

The companion target matrix is:

| Platform | Architecture | Rust target |
|---|---|---|
| GNU/Linux | x86-64 | `x86_64-unknown-linux-gnu` |
| GNU/Linux | ARM64 | `aarch64-unknown-linux-gnu` |
| macOS | Intel | `x86_64-apple-darwin` |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` |

Release support requires real helper discovery, synthesis, PCM validation,
cancellation or helper retirement, and clean shutdown on the native runner;
compiling a target alone is not treated as runtime acceptance.

## Build locally

Install the general Omnivox prerequisites plus a C compiler for the target.
Python downloads the checksum-locked source on the first build; preparation
and Cargo builds can then run from the verified cache:

```sh
make prepare-flite
python3 tools/prepare_flite_inputs.py --check
make stage-flite
```

`make build` and `make dev` also stage the release or debug companion beside
the main executable. `make install` copies that `flite/` directory into
`OMNIVOX_INSTALL_BIN`, which defaults to `~/.cargo/bin`.

The source preparer accepts `--output DIRECTORY`. The build wrapper uses
`OMNIVOX_FLITE_INPUTS_DIR` for the same cache override. The native Cargo build
never accesses the network; an advanced direct build must point
`OMNIVOX_FLITE_SOURCE_DIR` at the already verified extracted v2.2 source.

## Use the built-in voice

Select Flite initially with `--engine flite` or `OMNIVOX_ENGINE=flite`. The
physical voice ID is `cmu_us_slt`. It is a compact US English fallback with an
ASCII input guarantee. Flite provides rate, average-pitch, and volume control
but no word, sentence, or phoneme markers. eSpeak NG remains the final
Unicode-capable fallback.

For a quick native acceptance check:

```sh
target/release/omnivox --engine flite --list-voices
target/release/omnivox --engine flite \
  --dump-wav cmu_us_slt flite-test.wav "Flite is ready."
python3 tools/stress_helper.py \
  target/release/flite/omnivox-flite-helper \
  --engine-id flite --iterations 25 --cancel-probe \
  --require-acss rate --require-acss average_pitch --require-acss volume
```

On Windows, use the `.exe` helper path. `--cancel-probe` accepts either native
completion followed by cancellation or process retirement; Flite v2.2 has no
cooperative cancellation callback inside one synthesis call.

## Add local `.flitevox` voices

Set `OMNIVOX_FLITE_VOICES` to a platform path list of absolute `.flitevox`
files. Use `:` between paths on Linux and macOS and `;` on Windows:

```sh
export OMNIVOX_FLITE_VOICES="/opt/voices/one.flitevox:/opt/voices/two.flitevox"
```

```powershell
$env:OMNIVOX_FLITE_VOICES = 'C:\Voices\one.flitevox;D:\Voices\two.flitevox'
```

Omnivox does not scan directories or fetch these files. Paths must resolve to
regular files, and the files must contain Flite v2.2 Clustergen voices whose
language is `eng` or `usenglish`; those are the only external voice language
initializers compiled into this SLT-only companion. A loaded voice is reported
as `flitevox:INTERNAL_NAME`. A rejected path or duplicate internal name marks
the engine degraded while preserving the built-in SLT voice.

Treat `.flitevox` files as native engine input. A malformed file can terminate
the isolated helper, after which Omnivox can fall back and recreate it. Only
install files from a source you trust, and review each voice's licence before
redistribution. Optional voice files are never included in Omnivox releases.

## Provenance, licensing, and removal

Each binary companion includes its payload checksums, exact target and source
hashes, the Omnivox commit, the Flite source lock, Omnivox licensing map, and
Flite's complete upstream `COPYING` text. The separate Flite source artifact
contains the pinned upstream archive and the exact Omnivox build integration
needed to reproduce the helper.

Flite uses a BSD-like licence with attribution and naming conditions. The
Omnivox wrapper remains MIT-licensed; neither licence overrides the terms of a
user-supplied voice file.

To remove Flite, unset `OMNIVOX_FLITE_HELPER` and
`OMNIVOX_FLITE_VOICES`, delete the adjacent `flite/` directory, and restart
Omnivox. The main server and its other engines continue to work without the
companion.
