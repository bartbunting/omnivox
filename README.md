# Omnivox

Omnivox is a cross-platform speech server written in Rust. It implements the
[legacy Emacspeak line protocol](docs/protocols/LEGACY-PROTOCOL.md) and the
capability-gated
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

Current supported `make build`, `make dev`, and generic release builds enable
only Piper helper discovery in the main server. They do not link libpiper or
include its companion. `make build-piper` builds and stages that separate
native payload when it is wanted.

Server mode registers all available built-in engines for runtime routing and
fallback. Windows retains WinRT and eSpeak plus configured proprietary
helpers; macOS retains AVSpeechSynthesizer and eSpeak; Linux retains eSpeak.
A Piper-enabled server also registers its helper on any platform when a model
is configured. `--engine` changes the initial preference without hiding the
other registered engines. Registration makes an engine available to routing;
it does not promise that engines synthesize in parallel.

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

See [STATUS.md](docs/STATUS.md) for current limitations and
[ARCHITECTURE.md](docs/ARCHITECTURE.md)
for ownership and data flow.

## Binary releases

[GitHub Releases](https://github.com/bartbunting/omnivox/releases) provides
native archives for Linux x64, macOS Apple Silicon and Intel, and Windows x64
and ARM64. Archives produced by the current workflow contain the executable,
the matching generated `espeak-ng-data`, project and third-party licensing
files, and the upstream Emacspeak adapter. A `sha256sums.txt` file is published
alongside them. Published release `v1.4.1` predates the root `LICENSE` and
`LICENSING.md` archive entries; those files are included beginning with the
next release. Linux ARM64 is not currently published or CI-verified.

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

The Piper native build uses the vendored `v1.7.0` C API from the maintained
[`OHF-Voice/piper1-gpl`](https://github.com/OHF-Voice/piper1-gpl) project. It
requires CMake 3.26, a C++17-capable compiler, and currently requires network
access on the first build to download target-specific, checksum-locked eSpeak
NG, Sonic, and ONNX Runtime 1.22.0 inputs. Repeated native builds verify that
cache and can run offline. `make build-piper` stages an isolated
`target/release/piper/`
directory with the helper, native libraries, matching eSpeak data, notices,
provenance, and checksums. `make package-piper` creates a deterministic native
companion candidate: `.tar.gz` for Linux/macOS and `.zip` for Windows.
`make verify-piper` verifies the extracted runtime. Set `PIPER_MODEL` to a
licence-reviewed local model to add real end-to-end synthesis. Linux x64,
Windows x64, and both macOS architectures now pass native staging,
deterministic archive verification, and CI-only real synthesis.
Corresponding-source review, test-model licensing review, and release-workflow
integration remain open, so no Piper archive is published yet. Native Piper
code remains confined to the helper executable.

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
same profile directory as the executable, together with `LICENSE` and
`LICENSING.md`. Direct `cargo build` remains useful for compiler diagnostics
but does not create that complete runtime payload.

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

See [ENV-VARS.md](docs/ENV-VARS.md) for the complete CLI and environment
reference.

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

## Documentation

The [documentation index](docs/README.md) groups maintained references,
protocol specifications, operational guides, active plans, and historical
material. Start with [STATUS.md](docs/STATUS.md) for current support and
limitations, or [ENV-VARS.md](docs/ENV-VARS.md) for configuration.

## Licensing

Omnivox-authored source is available under the [MIT License](LICENSE), except
where a file carries another notice. The
[component licensing map](docs/LICENSING.md) explains the separately licensed
Emacspeak adapter, eSpeak NG, optional Piper integration, proprietary runtimes,
and other dependencies.

Distributed executables statically incorporate GPL-3.0-or-later eSpeak NG and
must be conveyed in compliance with the applicable terms for that combined
binary. Supported builds package the eSpeak NG data and applicable GPL,
Unicode, NetBSD, and Sonic notices in `third-party-licenses`; preserve that
directory when redistributing the payload.
