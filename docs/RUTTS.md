# RuTTS Companion Guide

Omnivox ships RuTTS as a separate, source-built companion process. The
companion statically contains RuTTS v6.3.3 and its built-in `male` and
`female` Russian voices. Users do not need a system RuTTS installation or
separate voice data, and Omnivox does not download data at run time.

The build is pinned to upstream commit
`2848d2892097320ed37fc963b439b15803f47f0c`. Omnivox verifies both the source
archive and complete extracted tree before compiling the dictionary-free
library. RuLex is not included or loaded; see
[Pronunciation and text support](#pronunciation-and-text-support).

## Install a release companion

RuTTS is intentionally separate from the generic Omnivox archive. Download
the RuTTS companion whose platform and architecture match the generic archive,
verify both with the release `sha256sums.txt`, and extract its one top-level
`rutts/` directory beside `omnivox` or `omnivox.exe`:

```text
installation-directory/
  omnivox                 # omnivox.exe on Windows
  rutts/
    omnivox-rutts-helper  # .exe on Windows
    README.md
    SOURCE-PROVENANCE.json
    SHA256SUMS
    third-party-licenses/
```

The main server discovers that layout automatically. An explicit helper path
can instead be set with `OMNIVOX_RUTTS_HELPER`.

The companion target matrix is:

| Platform | Architecture | Rust target |
|---|---|---|
| GNU/Linux | x86-64 | `x86_64-unknown-linux-gnu` |
| GNU/Linux | ARM64 | `aarch64-unknown-linux-gnu` |
| macOS | Intel | `x86_64-apple-darwin` |
| macOS | Apple Silicon | `aarch64-apple-darwin` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` |

Release support requires helper discovery, real synthesis, PCM validation,
cancellation, and clean shutdown on the native runner. Compiling a target
alone is not runtime acceptance. Windows release candidates use MSVC;
`x86_64-pc-windows-gnu` is additionally supported for Emacsvox's pinned MinGW
development build and records the distinct `windows-x64-gnu` target in its
provenance. That GNU target has passed a complete WSL-to-Windows build plus
male/female synthesis, ACSS, cancellation, and shutdown acceptance. The two
MSVC release targets remain subject to their native workflow gates.

## Build locally

Install the general Omnivox prerequisites plus a C compiler for the target.
Python downloads the checksum-locked source on the first build; preparation
and Cargo builds can then run from the verified cache:

```sh
make prepare-rutts
python3 tools/prepare_rutts_inputs.py --check
make stage-rutts
```

`make build` and `make dev` also stage the release or debug companion beside
the main executable. `make install` copies that `rutts/` directory into
`OMNIVOX_INSTALL_BIN`, which defaults to `~/.cargo/bin`.

The source preparer accepts `--output DIRECTORY`. The build wrapper uses
`OMNIVOX_RUTTS_INPUTS_DIR` for the same cache override. Its native Cargo build
does not access the network; an advanced direct build must point
`OMNIVOX_RUTTS_SOURCE_DIR` at the already verified v6.3.3 source tree.

## Use the built-in voices

Select RuTTS initially with `--engine rutts` or `OMNIVOX_ENGINE=rutts`. Its
physical voice IDs are `male` and `female`, both reported as `ru-RU`. The
companion provides rate, average-pitch, pitch-range/intonation, and volume
control. It does not provide word, sentence, or phoneme markers.

For a quick native acceptance check:

```sh
target/release/omnivox --engine rutts --list-voices
target/release/omnivox --engine rutts \
  --dump-wav male rutts-test.wav "Привет, мир!"
python3 tools/stress_helper.py \
  target/release/rutts/omnivox-rutts-helper \
  --engine-id rutts --voice-id male --iterations 25 --cancel-probe \
  --require-acss rate --require-acss average_pitch \
  --require-acss pitch_range --require-acss volume
```

On Windows, use the `.exe` helper path. Repeat with `--voice-id female` to
exercise the other built-in voice.

## Pronunciation and text support

The upstream API accepts KOI8-R, not UTF-8. The helper converts ASCII, Russian
Cyrillic, and `Ё`/`ё` losslessly at its process boundary. Text containing a
character outside that repertoire, such as an em dash or Ukrainian letter,
is rejected before the native call so Omnivox can route the unchanged chunk to
a Unicode-capable fallback such as eSpeak NG.

Without RuLex, RuTTS applies its built-in pronunciation rules but has no
external stress or exception dictionary. Upstream's input syntax treats `+`
immediately after a vowel as strong stress and `=` as weak stress, so callers
can annotate ambiguous words explicitly. Those ASCII annotations are passed
through the helper.

RuLex remains a separate future decision because it adds an LGPL library,
dictionary database, storage backend, locale requirements, and additional
cross-platform packaging. Installing RuLex on the host does not change this
companion.

## Provenance, licensing, and removal

Each binary companion includes payload checksums, exact target and source
hashes, the Omnivox commit, the source-input lock, the complete upstream MIT
licence, and Omnivox's licensing map. The separate RuTTS source artifact
contains the pinned upstream archive and exact Omnivox build integration needed
to reproduce the helper. The built-in voice data is part of that upstream
source.

The Omnivox wrapper remains MIT-licensed. RuLex is not present in either the
binary or source companion.

To remove RuTTS, unset `OMNIVOX_RUTTS_HELPER`, delete the adjacent `rutts/`
directory, and restart Omnivox. The main server and its other engines continue
to work without the companion.
