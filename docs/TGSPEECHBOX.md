# TGSpeechBox Experimental Companion

TGSpeechBox is an optional formant-synthesis engine with a compact,
DECtalk-like sound and direct rate, pitch, inflection, volume, and profile
controls. Omnivox pins the exact upstream `v-310` branch snapshot
`v-310@f5ec247` at commit `f5ec247bca50507ab1e2ed661136395538dc3e97`
and keeps its C++ frontend and DSP inside a separate helper process. This
snapshot is six commits after `v-310b802`; it is not presented as an upstream
release tag.

Beginning with Omnivox v1.7.0, Windows x64 GNU is published as a separate
experimental companion. It is not embedded in generic archives or the
Emacsvox bundle. Linux x64 remains a development smoke target. The helper has
a measured rate curve and exact caller-requested synchronization anchors; its
upstream beta status and limited runtime-accepted platform coverage still make
the companion experimental.

## What is exposed

The accepted Windows build reports 154 physical voices: seven profiles across
22 language packs shared by TGSpeechBox and eSpeak NG.

- Profiles: Adam, Benjamin, Caleb, David, Robert, Beth, and Bobby.
- Languages: `bg`, `cs`, `da`, `de`, `en`, `en-us`, `es`, `fi`, `fr`, `hr`,
  `hu`, `it`, `nl`, `pl`, `pt`, `pt-br`, `ro`, `ru`, `sk`, `sv`, `tr`, and
  `uk`.
- Voice IDs combine both parts, for example `en-us/adam`, `de/robert`, or
  `ru/caleb`.

Adam, Benjamin, Caleb, David, and Robert are built-in parameter profiles. Beth
and Bobby are data-defined profiles loaded by the pinned frontend after
language selection. Omnivox advertises only profiles the frontend enumerates
and accepts.

The helper accepts Omnivox's portable rate, average-pitch, pitch-range, and
volume dimensions. Its Adam `en-us` calibration follows the Eloquence `v1`
reference curve through host rate `1.0`, then reaches TGSpeechBox's native
ceiling:

| Omnivox rate | TGSpeechBox speed |
|---|---:|
| `0.0` | `0.312017x` |
| `0.5` | `0.963270x` |
| `0.8` | `2.137347x` |
| `1.0` | `3.275296x` |
| `1.2`–`2.0` | `4x` (native ceiling) |

Average pitch maps to a 110 Hz native baseline, pitch range maps to native
inflection, and volume maps to output gain. TGSpeechBox-specific frame fields
are not added to the public protocol.

The selected upstream revision exposes an index-aware DSP pull. For an
anchored request, Omnivox divides the original UTF-8 text only at the explicitly
requested offsets, gives the following frontend segment an opaque native index,
and reports that index as an exact requested anchor at the corresponding PCM
boundary. Anchors at the beginning or end resolve to frame zero or the final
frame. Internal non-punctuated segments use continuation intonation rather than
an artificial sentence ending. Omnivox does not claim general word, sentence,
phoneme, or native-index markers: the TGSpeechBox frontend does not map those
generated frames back to truthful source-text ranges.

## Build Windows x64 from WSL

The first build downloads and verifies the locked TGSpeechBox archive. Later
builds reuse the verified cache and can check it without downloading again.

```sh
make prepare-tgspeechbox
python3 tools/prepare_tgspeechbox_inputs.py --check
make build-tgspeechbox-windows
make verify-tgspeechbox
make verify-tgspeechbox-source
```

The complete companion is staged at:

```text
target/x86_64-pc-windows-gnu/release/tgspeechbox/
```

It contains `omnivox-tgspeechbox-helper.exe`, TGSpeechBox packs, generated
`espeak-ng-data`, helper-generated rate-specific voice inventories plus the
default-compatible `VOICE-INVENTORY.json`, licence notices, exact source
provenance, and `SHA256SUMS`. The MinGW C++ runtime is linked statically, so the
helper does not require a separately staged `libstdc++-6.dll`.

`make verify-tgspeechbox` creates and exercises
`omnivox-VERSION-tgspeechbox-windows-x64.zip`. The tag workflow repeats that
check on Windows and combines the relocated companion with the exact generic
Windows archive for full-server voice discovery and WAV synthesis.
`make verify-tgspeechbox-source` creates and verifies the platform-neutral
`omnivox-VERSION-tgspeechbox-source.tar.gz`, including the exact Omnivox tree,
vendored Cargo/eSpeak NG source, locked TGSpeechBox archive, and exhaustive
manifest.

To test discovery through a Windows Omnivox executable built from the active
checkout:

```sh
cargo build --locked --release --package omnivox-cli \
  --target x86_64-pc-windows-gnu

target/x86_64-pc-windows-gnu/release/omnivox.exe \
  --list-voices --engine tgspeechbox

target/x86_64-pc-windows-gnu/release/omnivox.exe \
  --check --engine tgspeechbox --voice en-us/adam
```

The interactive check prints the exact engine and voice used, synthesizes and
plays speech, and tests the Windows audio path. It must report
`engine tgspeechbox, voice en-us/adam`; process exit alone is not the complete
acceptance criterion.

For a noninteractive helper-protocol test from WSL:

```sh
python3 tools/stress_helper.py \
  target/x86_64-pc-windows-gnu/release/tgspeechbox/omnivox-tgspeechbox-helper.exe \
  --engine-id tgspeechbox --iterations 25 --cancel-probe --health-every 5 \
  --require-acss rate --require-acss average_pitch \
  --require-acss pitch_range --require-acss volume --require-streaming
```

## Runtime layout and configuration

Place the complete `tgspeechbox/` directory beside `omnivox.exe`. Omnivox
discovers `tgspeechbox/omnivox-tgspeechbox-helper.exe` automatically. An
explicit absolute helper path can override discovery:

```text
OMNIVOX_TGSPEECHBOX_HELPER
```

The helper normally reads `packs/` and `espeak-ng-data/` beside itself.
`OMNIVOX_TGSPEECHBOX_DATA` may name an absolute directory containing
`packs/phonemes.yaml`; `ESPEAK_NG_DATA` may name an absolute parent directory
containing `espeak-ng-data/phontab`. Keep the staged companion intact when
relocating it.

In server mode, `--engine tgspeechbox` or `OMNIVOX_ENGINE=tgspeechbox` makes it
the initial preference while retaining other available engines as fallbacks.
An exact diagnostic selection fails if the helper is missing or incomplete.

Server mode selects the inventory matching the configured native sample rate,
validates it, and uses it to register all 154 voices without blocking startup
on native initialization. After validating the cache, Omnivox opens and retains
the TGSpeechBox helper connection on a background thread while other engines
continue initializing. If speech arrives before pre-warming finishes, that
request waits for the same connection instead of starting a second helper. A
missing, oversized, malformed, or mismatched cache causes eager live
initialization instead. The persistent helper also retains the last
successfully configured frontend language, profile, and eSpeak voice, avoiding
the pack reload and duplicate voice selection when successive utterances use
the same physical voice.

At the default 44.1 kHz native rate, helper protocol v5 forwards each bounded
DSP pull while synthesis is still active. This removes whole-utterance capture
from the time-to-first-audio path without changing sample rate or adding a
resampling boundary. Omnivox relays ordinary speech and exact requested anchors
through bounded isolation and a single tracked playback source while applying
the same silence trimming, effects, volume, and channel routing across windows.
Capitalization tones and timed presentation actions therefore remain
progressive. Older Omnivox clients negotiate the buffered path.

TGSpeechBox normally runs its native DSP at 44.1 kHz. For controlled A/B tests,
`OMNIVOX_TGSPEECHBOX_SAMPLE_RATE=22050` selects its supported 22.05 kHz path;
canonical Omnivox output remains stereo 44.1 kHz. The setting changes the live
engine descriptor. The staged companion carries matching 44.1 and 22.05 kHz
inventories, so switching needs only the environment change and a server
restart; rebuilding is unnecessary. The 22.05 kHz mode remains experimental
until timing and listening results justify changing the default. It deliberately
remains buffered so the existing whole-utterance sinc conversion is not replaced
by lower-quality independent resampling at every native pull boundary.

## Licensing and removal

TGSpeechBox is MIT-licensed. The helper statically incorporates GPLv3 eSpeak
NG for Unicode-to-IPA conversion, so the combined helper is distributed under
GPLv3 and its complete notices must remain with it. The separately published
corresponding-source artifact contains the complete inputs needed for the
combined helper. See [LICENSING.md](LICENSING.md) and
[ADR 0005](adr/0005-experimental-tgspeechbox-companion.md) together with its
[calibration and marker refinement](adr/0007-tgspeechbox-calibration-and-anchors.md).

Remove the `tgspeechbox/` directory and unset
`OMNIVOX_TGSPEECHBOX_HELPER` to remove the engine. Omnivox continues with its
remaining engines.
