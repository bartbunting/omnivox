# Changelog

This file records user-visible changes to Omnivox. It follows the structure of
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses Semantic
Versioning for published releases.

## Unreleased

### Changed

- Reduced the Eloquence helper's synthesis idle timeout from three seconds to
  500 milliseconds so a wedged native synchronization call reaches configured
  fallback promptly.
- Server startup now retains every available built-in engine in the runtime
  registry. In particular, macOS keeps both AVSpeechSynthesizer and eSpeak NG;
  Piper-enabled builds register a configured Piper helper without requiring it
  to be the selected startup engine.
- Standard Makefile and generic release builds now enable Piper helper
  discovery in the main server without linking or packaging Piper native code.
- Migrated the experimental Piper helper from the archived custom C++ bridge to
  the vendored maintained libpiper v1.7 C API. Piper now consumes chunked float
  audio and observes cancellation between chunks while retaining process
  retirement for blocked native calls.
- Piper-enabled servers now prefer an isolated `piper/` companion directory,
  while retaining the explicit helper override and legacy adjacent-helper
  lookup. The helper prefers the matching eSpeak data inside that directory.
- Added atomic Linux x64 Piper companion staging with relative native-library
  lookup, matching eSpeak data, notices, provenance, and payload checksums.
- Piper's Linux x64 build now verifies locked eSpeak NG, Sonic, and ONNX
  Runtime archives before CMake uses them and supports repeat builds with
  network access disabled.
- Added deterministic `omnivox-VERSION-piper-linux-x64.tar.gz` construction
  and fail-closed verification of its layout, checksums, provenance,
  architecture, dynamic linkage, relocation, and optional real synthesis.
- Added checksum-locked Piper native-input manifests for Windows x64 and
  macOS ARM64/x64, including safe ZIP extraction and Windows-compatible
  materialization of the eSpeak source link.
- Generalized Piper companion staging for the Windows x64 and macOS
  ARM64/x64 helper and library layouts, with native PE or Mach-O validation.
- Generalized deterministic Piper packaging and relocated-runtime verification
  for Linux x64, Windows x64, and macOS ARM64/x64 native artifacts.
- Added a manual, non-publishing native-runner workflow; Linux x64, Windows
  x64, and both macOS architectures now build and stage, recheck locked inputs,
  verify relocated deterministic archives, synthesize real audio, exercise a
  persistent 25-request session, and cancel an in-flight request without stale
  output.
- Completed native Windows x64 Piper build and staging, including offline input
  rechecks and PE validation.
- Locked the upstream libpiper English test model, configuration, and model
  card for CI-only synthesis without adding the model to release artifacts;
  its licensing review remains explicitly deferred.

### Fixed

- Kept the Piper helper's libpiper eSpeak runtime separate from the main
  server's eSpeak backend, preventing ELF symbol interposition from crashing
  Linux synthesis.

### Documentation

- Corrected the documented release matrix, archive installation steps, engine
  registration behavior, CLI diagnostics, logging retention, and Piper build
  limitations.
- Added complete Emacspeak setup and legacy line-protocol references.
- Added control, timeline, marker-event, and helper-protocol examples that are
  checked against the public Rust wire types in the test suite.
- Added the missing root MIT license and a component map that distinguishes
  Omnivox source licensing from adapter, eSpeak-linked binary, Piper, and
  proprietary-runtime terms; future binary archives include and validate both
  project licensing files.
- Moved maintained references and protocols under an indexed `docs/` hierarchy,
  with active plans and historical notes clearly separated from shipped
  behavior.

## 1.4.1 - 2026-08-30

This was the first published 1.4 release. The `v1.4.0` workflow created a draft
release but did not pass native archive verification; that draft was
superseded by `v1.4.1` and its assets should not be used.

### Added

- A versioned control protocol for engine inventory, logical-voice
  registration, runtime routing policy, recovery probes, and exact voice
  previews.
- Deterministic multi-engine routing with text-repertoire fallback, persistent
  health circuits, and recovery across eSpeak NG, WinRT, AVSpeechSynthesizer,
  and configured helper engines.
- Structured presentation timelines with speech spans, inserted and overlaid
  audio, tones, silence, semantic events, persistent effects, multipart
  version 3 transport, replacement domains, and tracked terminal status.
- Playback-synchronized engine markers, requested anchors, action-resolution
  events, and style-degradation events where the selected engine supports
  them.
- A versioned out-of-process helper protocol used by the optional Eloquence,
  DECtalk, and Piper integrations.
- Rotated persistent diagnostics with bounded retention and opt-in synthesis
  text logging.

### Changed

- Bounded protocol admission, synthesis handoff, reporting queues, timeline
  resources, and prepared audio to contain malformed or excessive work.
- Isolated uncancellable native synthesis and strengthened helper cancellation,
  failure recovery, and stale-output suppression.
- Staged the exact generated eSpeak NG runtime data and third-party notices
  beside supported builds and in every binary release archive.
- Added native CI build, test, Clippy, package, relocation, architecture,
  voice-discovery, and WAV-synthesis verification for five release targets.

### Fixed

- Kept the macOS command-line process alive while asynchronous native synthesis
  is active.
- Corrected punctuation boundaries, capitalization presentation, cancellation
  fades, voice identity normalization, and several structured-timeline timing
  and replacement races.
- Added recovery for a draft release whose native verification needs to be
  rerun without rebuilding or replacing its uploaded assets.

[Unreleased]: https://github.com/bartbunting/omnivox/compare/v1.4.1...HEAD
[1.4.1]: https://github.com/bartbunting/omnivox/compare/v1.3.0...v1.4.1
