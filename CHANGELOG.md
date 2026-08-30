# Changelog

This file records user-visible changes to Omnivox. It follows the structure of
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses Semantic
Versioning for published releases.

## Unreleased

### Changed

- Reduced the Eloquence helper's synthesis idle timeout from three seconds to
  500 milliseconds so a wedged native synchronization call reaches configured
  fallback promptly.

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
