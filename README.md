# Omnivox

Omnivox is a cross-platform speech server written in Rust. It implements the
[legacy Emacspeak line protocol](LEGACY-PROTOCOL.md) and the capability-gated
structured protocols used by Emacsvox for logical voices, tracked playback,
marker events, and Aural presentation timelines.

## Integration choices

There are two supported consumers with deliberately different Lisp adapters:

- **Emacsvox** uses its bundled `lisp/omnivox-voices.el`. This is the complete
  integration: capability negotiation, logical-voice routing, structured
  timelines, replacement domains, playback events, and the Voice Workbench.
  On Windows under WSL, build and stage Omnivox from the Emacsvox repository.
- **Upstream Emacspeak** can use this repository's
  `elisp/omnivox-voices.el`. That adapter uses Emacspeak's established `dtk-*`
  API names and the compatible line protocol. Do not copy its configuration
  variable names into an Emacsvox setup.

## Current engines

| Platform | Default/native engine | Other available engines |
|---|---|---|
| macOS | AVSpeechSynthesizer | eSpeak NG; optional Piper helper |
| Windows | WinRT SpeechSynthesizer | eSpeak NG; optional Eloquence, DECtalk, and Piper helpers |
| Linux | eSpeak NG | optional Piper helper |

The Windows Eloquence and DECtalk engines run in separate 32-bit helper
processes and require user-supplied proprietary runtimes. Omnivox and Emacsvox
do not distribute those runtimes. They are discovered as helper engines for
runtime routing rather than selected with the startup `--engine` option. The
Piper backend is opt-in and is built separately so its native dependencies
never enter the main server process. Speech Dispatcher remains a design
proposal, not an implemented backend.

Windows server processes register available WinRT, eSpeak, and configured
proprietary helpers for runtime routing and fallback. macOS and Linux currently
register only the selected startup engine, so choosing `--engine espeak` or
`--engine piper` there does not retain another engine as an in-process fallback.

The eSpeak backend is compiled from source. Supported local builds stage the
matching generated voice data beside the executable, and generic GitHub release
archives include that `espeak-ng-data` directory and its third-party notices.
Keep those directories together when relocating a build. `ESPEAK_NG_DATA` can
still select another compatible data parent explicitly. Emacsvox's Windows
release path independently stages and records its pinned runtime inputs.

## Highlights

- Bounded, nonblocking protocol admission and synthesis handoff.
- Deterministic logical-voice resolution across engines and physical voices,
  with runtime fallback, health circuits, and recovery probes.
- Canonical stereo 44.1 kHz PCM processing, silence trimming, volume and
  channel control, and post-synthesis filtering, reverb, and echo.
- Structured Aural timelines with speech spans, inserted or overlaid audio and
  tones, semantic events, persistent effects, multipart transport, and tracked
  terminal status.
- Domain-scoped replacement: admission of a newer keyed presentation
  atomically cancels queued, synthesizing, and buffered work in the same
  replacement domain without clearing ordered, urgent, or unrelated output.
  Failed admission leaves the older work intact. Active speech uses a short
  de-click fade.
- Playback-synchronized word, sentence, phoneme, native-index, semantic, and
  degradation events where the selected engine can provide them.
- Generation-aware isolation for native synthesis calls that cannot be
  interrupted safely, and killable helper processes where process isolation
  permits stronger cancellation.
- Bounded audio-resource decoding and immutable shared PCM for timeline reuse.
- Persistent privacy-conscious diagnostics; synthesis text is excluded unless
  the user explicitly opts in, while operational metadata remains available.

See [STATUS.md](STATUS.md) for current limitations and [ARCHITECTURE.md](ARCHITECTURE.md)
for ownership and data flow.

## Binary releases

[GitHub Releases](https://github.com/bartbunting/omnivox/releases) provides
native archives for Linux x64, macOS Apple Silicon and Intel, and Windows x64
and ARM64. Each archive contains the executable, the matching generated
`espeak-ng-data`, third-party notices, and the upstream Emacspeak adapter. A
`sha256sums.txt` file is published alongside them. Linux ARM64 is not currently
published or CI-verified.

Follow the [release and deployment guide](.github/DEPLOYMENT.md) for archive
selection, checksum commands, installation, unsigned-binary warnings, and the
separate Emacsvox Windows deployment contract. Keep the data and notices beside
the executable after extraction. User-visible release changes are recorded in
[CHANGELOG.md](CHANGELOG.md).

## Prerequisites

- [rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects
  the exact supported Rust release.
- Python 3, used by the supported build wrapper to stage the eSpeak NG data
  generated by Cargo's locked dependency build.
- A C/C++ toolchain, CMake, and Clang/libclang available to bindgen for the
  eSpeak NG backend build.
- On Linux, `pkg-config` and ALSA development headers (`libasound2-dev` on
  Debian/Ubuntu or `alsa-lib-devel` on Fedora) for the rodio/cpal audio backend.
- Platform build tools: Xcode command-line tools on macOS or Visual Studio
  build tools on Windows.

Normal Cargo entry points are locked because `Cargo.lock` is part of the
application release contract. Do not substitute an unpinned Rust toolchain or
unlocked dependency resolution when diagnosing the repository.

## Build and validation

```sh
make build       # locked release build plus adjacent eSpeak runtime data
make dev         # locked debug build plus adjacent eSpeak runtime data
make fmt-check   # non-mutating format check
make test        # locked default-member tests
make lint        # locked Clippy with warnings denied
```

The full workspace check, including non-default workspace members, is:

```sh
cargo test --locked --workspace
```

Build the optional Piper helper and Piper-enabled server together with:

```sh
make build-piper
```

The first Piper native build requires network access to fetch its native inputs
and a C++17-capable compiler. The current build fetches the archived
[`rhasspy/piper`](https://github.com/rhasspy/piper) source and the
[`piper-phonemize` master branch](https://github.com/rhasspy/piper-phonemize/tree/master)
rather than pinned revisions. The archived project points to the newer
[`OHF-Voice/piper1-gpl`](https://github.com/OHF-Voice/piper1-gpl) project, but
Omnivox has not yet migrated or established compatibility with it. Treat the
current integration as an experimental developer build, not a reproducible
release payload. Native Piper code remains confined to the helper executable;
dependency pinning, native-library staging, notices, and model distribution are
still tracked as release work.

Running Piper requires a compatible `.onnx` model and its adjacent
configuration, named either `<model>.onnx.json` or `<model>.json`. Model
compatibility and licensing depend on the model source; Omnivox does not
currently distribute or endorse a model catalogue.

The test suite changes frequently, so documentation does not embed a test
count. A passing suite establishes correctness coverage; it is not a latency
measurement.

`tools/build.py` owns the distributable build step because Cargo has no
reliable post-build hook: it runs a locked Cargo build, identifies the actual
`espeak-rs-sys` output reported by Cargo, and stages its data and notices in the
same profile directory as the executable. Direct `cargo build` remains useful
for compiler diagnostics but does not create that complete runtime payload.

## Emacsvox on Windows under WSL

The Emacsvox repository owns the reproducible Windows bundle, helper builds,
runtime provenance, and content-addressed launcher selection:

```sh
cd /path/to/emacsvox
make windows-omnivox
```

The release target requires clean tracked worktrees. For local testing of
active changes, use `make windows-omnivox-dev`; it records both repositories'
tracked-diff hashes in the staged provenance. In Emacsvox, select the server
with `M-x omnivox`. Restart Emacsvox after staging a new content-addressed
runtime; existing speech-server processes continue running the executable
with which they started.

The full release contract is in the Emacsvox document
`servers/omnivox-release/README.org`.

## Standalone Emacspeak integration

Build and install the binary:

```sh
make build
make install
```

`make install` keeps `espeak-ng-data` and `third-party-licenses` beside the
installed executable. It defaults to `~/.cargo/bin`; set
`OMNIVOX_INSTALL_BIN` when Cargo installs binaries elsewhere.

Make `omnivox` discoverable on `PATH` or place it in Emacspeak's `servers`
directory. When Omnivox should own auditory-icon playback, add a line containing
only `omnivox` to `/path/to/emacspeak/servers/.servers`:

```text
omnivox
```

The adapter requires `emacspeak-preamble`, so the Emacspeak `lisp` directory
must already be on `load-path`. Load the adapter after that prerequisite is
available but before `emacspeak-setup` completes:

```elisp
(add-to-list 'load-path "/path/to/omnivox/elisp") ; source checkout
;; For the archive layout in the deployment guide, use:
;; (add-to-list 'load-path "~/.emacs.d/lisp/omnivox")
(require 'omnivox-voices)

;; Omnivox uses a 0--100 rate scale; larger values are faster.
(setq omnivox-speech-rate 60
      omnivox-voice-id "")       ; Empty means the engine default.

(setq dtk-program "omnivox")
(require 'emacspeak-setup)
```

These `dtk-*` names belong to upstream Emacspeak. Emacsvox intentionally uses
the generic `tts-*` namespace and its own Omnivox adapter. The compatibility
module tracks current Emacspeak APIs; the project does not yet claim a minimum
Emacs or Emacspeak version, so test it with the checkout you intend to deploy.

## Command-line use

Run `omnivox --help` for the authoritative list. Common commands include:

```sh
omnivox --check
omnivox --list-voices
omnivox --engine espeak --rate 0.6
omnivox --voice-volume 1.0 --tone-volume 0.1 --sound-volume 0.5
omnivox --audio-target left
omnivox --dump-wav VOICE output.wav "Text to synthesize"
```

Piper additionally uses `--engine piper --piper-model /path/to/model.onnx`,
with the matching JSON configuration beside the model. Without an action
option, Omnivox runs the stdin speech-server protocol.

See [ENV-VARS.md](ENV-VARS.md) for the complete CLI and environment reference.

## Diagnostics

`omnivox --check` exercises engine and audio initialization. The Emacsvox WSL
launcher also writes private, rotated log parts for each session. To collect a
bounded bundle without memory-dump contents:

```sh
tools/collect_diagnostics.sh
```

See [docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) before enabling full synthesis
text or Windows crash dumps; both can contain private spoken content.

Useful distinctions when investigating responsiveness:

- a protocol write or tracked terminal record is not proof of physical audio;
- playback marker events report mixer-source consumption and may precede the
  audible device output by its buffer latency;
- cold engine/model startup must be separated from warm interactive latency;
- replacement cancellation varies by engine, although stale PCM is never
  admitted after cancellation.

## Protocol and design documents

- [ARCHITECTURE.md](ARCHITECTURE.md) — runtime ownership and data flow.
- [LEGACY-PROTOCOL.md](LEGACY-PROTOCOL.md) — baseline Emacspeak command
  grammar, queue semantics, state, and limits.
- [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) — discovery, logical voices,
  routing policy, preview, tracked playback, and legacy framing.
- [PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md) —
  structured Aural timeline versions and multipart transport.
- [HELPER-PROTOCOL.md](HELPER-PROTOCOL.md) — isolated engine-host protocol.
- [ENV-VARS.md](ENV-VARS.md) — configuration and integration boundaries.
- [STATUS.md](STATUS.md) — implemented behavior and current limitations.
- [NEXT_STEPS.md](NEXT_STEPS.md) — current roadmap only.
- [docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) — failure-evidence workflow.
- [docs/ENGINE-ISOLATION.md](docs/ENGINE-ISOLATION.md) — uncancellable engine
  containment.
- [docs/TEXT-CHUNKING.md](docs/TEXT-CHUNKING.md) — current text chunking.
- [.github/DEPLOYMENT.md](.github/DEPLOYMENT.md) — release archives,
  verification, installation, and acceptance checks.
- [SPEECHD-PLAN.md](SPEECHD-PLAN.md) — unimplemented Speech Dispatcher design
  proposal.

`CHUNKING-IMPLEMENTATION.md` is retained as a historical implementation note
and points to the current chunking reference. Git history remains the source
for superseded phase-by-phase plans.

## Licensing

Omnivox-authored source is available under the [MIT License](LICENSE), except
where a file carries another notice. The
[component licensing map](LICENSING.md) explains the separately licensed
Emacspeak adapter, eSpeak NG, optional Piper integration, proprietary runtimes,
and other dependencies.

Distributed executables statically incorporate GPL-3.0-or-later eSpeak NG and
must be conveyed in compliance with the applicable terms for that combined
binary. Supported builds package the eSpeak NG data and applicable GPL,
Unicode, NetBSD, and Sonic notices in `third-party-licenses`; preserve that
directory when redistributing the payload.
