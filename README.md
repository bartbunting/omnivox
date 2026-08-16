# Omnivox

Omnivox is a cross-platform speech server written in Rust. It implements the
legacy Emacspeak line protocol and the capability-gated structured protocols
used by Emacsvox for logical voices, tracked playback, marker events, and Aural
presentation timelines.

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
do not distribute those runtimes. The Piper backend is opt-in and is built
separately so its native dependencies never enter the main server process.
Speech Dispatcher remains a design proposal, not an implemented backend.

## Highlights

- Bounded, nonblocking protocol admission and synthesis handoff.
- Deterministic logical-voice resolution across engines and physical voices,
  with runtime fallback, health circuits, and recovery probes.
- Canonical stereo 44.1 kHz PCM processing, silence trimming, volume and
  channel control, and post-synthesis filtering, reverb, and echo.
- Structured Aural timelines with speech spans, inserted or overlaid audio and
  tones, semantic events, persistent effects, multipart transport, and tracked
  terminal status.
- Domain-scoped replacement: a newer keyed presentation cancels queued,
  synthesizing, and buffered work in the same replacement domain without
  clearing ordered, urgent, or unrelated output. Active speech uses a short
  de-click fade.
- Playback-synchronized word, sentence, phoneme, native-index, semantic, and
  degradation events where the selected engine can provide them.
- Generation-aware isolation for native synthesis calls that cannot be
  interrupted safely, and killable helper processes where process isolation
  permits stronger cancellation.
- Bounded audio-resource decoding and immutable shared PCM for timeline reuse.
- Persistent privacy-safe diagnostics; synthesis text is excluded unless the
  user explicitly opts in.

See [STATUS.md](STATUS.md) for current limitations and [ARCHITECTURE.md](ARCHITECTURE.md)
for ownership and data flow.

## Prerequisites

- [rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects
  the exact supported Rust release.
- A C compiler and CMake for the bundled eSpeak NG build.
- Platform build tools: Xcode command-line tools on macOS or Visual Studio
  build tools on Windows.

Normal Cargo entry points are locked because `Cargo.lock` is part of the
application release contract. Do not substitute an unpinned Rust toolchain or
unlocked dependency resolution when diagnosing the repository.

## Build and validation

```sh
make build       # locked release build
make dev         # locked debug build
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

The test suite changes frequently, so documentation does not embed a test
count. A passing suite establishes correctness coverage; it is not a latency
measurement.

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

Make `omnivox` discoverable on `PATH` or place it in Emacspeak's server
directory. Add that server name to Emacspeak's `.servers` file when Omnivox
should own auditory-icon playback. Then load this repository's adapter before
Emacspeak:

```elisp
(add-to-list 'load-path "/path/to/omnivox/elisp")
(require 'omnivox-voices)

;; Omnivox uses a 0--100 rate scale; larger values are faster.
(setq omnivox-speech-rate 60
      omnivox-voice-id "")       ; Empty means the engine default.

(setq dtk-program "omnivox")
(require 'emacspeak-setup)
```

These `dtk-*` names belong to upstream Emacspeak. Emacsvox intentionally uses
the generic `tts-*` namespace and its own Omnivox adapter.

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

Piper additionally uses `--engine piper --piper-model /path/to/model.onnx`.
Without an action option, Omnivox runs the stdin speech-server protocol.

See [ENV-VARS.md](ENV-VARS.md) for the complete CLI and environment reference.

## Diagnostics

`omnivox --check` exercises engine and audio initialization. The Emacsvox WSL
launcher also writes one private log per session. To collect a bounded bundle
without memory-dump contents:

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
- [SPEECHD-PLAN.md](SPEECHD-PLAN.md) — unimplemented Speech Dispatcher design
  proposal.

`CHUNKING-IMPLEMENTATION.md` is retained as a historical implementation note
and points to the current chunking reference. Git history remains the source
for superseded phase-by-phase plans.

## Licensing

The Rust workspace declares the MIT license in `Cargo.toml`. The Emacspeak
adapter in `elisp/omnivox-voices.el` carries its own copyright and GNU GPL
notice in the file header.
