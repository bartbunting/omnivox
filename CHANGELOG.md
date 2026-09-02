# Changelog

This file records user-visible changes to Omnivox. It follows the structure of
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses Semantic
Versioning for published releases.

## [Unreleased]

Correction: v1.6.2 was not published. Its workflow stopped before draft
creation because a cross-platform Piper verifier selected each platform's
default diagnostic engine while requiring an eSpeak voice.

### Fixed

- Piper release verification now selects eSpeak explicitly when confirming
  that missing or corrupt Piper models leave the fallback engine usable on
  Linux, macOS, and Windows, with regression coverage for the command route.

## [1.6.2] - 2026-09-02

This is the first published 1.6 release. Correction to the prepared v1.6.1
entry below: v1.6.1 was not published. Its workflow stopped before draft
creation because the Piper verifier expected an exact-engine diagnostic to
fall back silently. v1.6.0 likewise published no release assets, as recorded
below.

### Fixed

- Piper release verification now requires missing and corrupt models to fail
  exact Piper diagnostics while ordinary discovery retains eSpeak, matching
  the server's exact-engine diagnostic contract instead of expecting silent
  fallback from an explicitly requested Piper engine.

## [1.6.1] - 2026-09-02

This is the first published 1.6 release. The `v1.6.0` workflow stopped before
draft creation because its clean source-packaging runners had not prepared the
locked Flite and RuTTS inputs. No v1.6.0 release assets were published.

### Fixed

- Flite and RuTTS corresponding-source release jobs now prepare and verify
  their checksum-locked upstream archives on their own clean runners before
  packaging; the matching local Make targets enforce the same prerequisite.

## [1.6.0] - 2026-09-02

### Added

- Added a standard-library cross-platform speech-rate audit that drives exact
  diagnostic engines through `--dump-wav`, measures raw and canonical WAV
  duration plus words per minute over repeated normalized rates, and can retain
  privacy-conscious JSON evidence without storing the spoken text.
- Added isolated RHVoice and Flite helpers with a shared bounded helper host.
  RHVoice loads a compatible user-installed 1.14-or-later 1.x C API runtime;
  Flite is reproducibly source-built from pinned v2.2 with only `cmu_us_slt`
  compiled in and optional local English `.flitevox` voice files.
- Added checksum-locked Flite source preparation, portable GCC/Clang/MSVC
  builds, atomic companion staging, full upstream licensing, provenance,
  payload checksums, real synthesis tests, and helper-session stress coverage.
- Added native Flite word-start markers and word-boundary resolution for
  caller-supplied synchronization anchors.
- Added dispatch-correlated, monotonic lifecycle telemetry for protocol
  admission, synthesis queue wait, engine attempts, audio queueing, first
  mixer-source consumption, and terminal playback without changing the marker
  or control protocols.
- Added a cross-platform cold/warm server benchmark for character, word, line,
  dense-action, multipart, and rapid-replacement workloads. Reports retain raw
  monotonic samples, build provenance, actual engines and physical voices, and
  p50/p95/p99 timing. Strict exact-voice selection and a KOI8-R-compatible
  Russian workload profile cover both RuTTS voices; a runtime preference option
  also exercises live policy replacement independently of startup selection.
- Added a bounded benchmark-suite plan runner that randomizes cross-engine order
  with a recorded seed, repeats complete runs, preserves separate raw reports
  and checksums, and refuses to overwrite an existing evidence directory.
- Added server-level stress tooling for interleaved replacement domains,
  ordered and urgent survivors, hard-stop recovery, marker and semantic-event
  ordering, exactly-once terminal history, and opt-in validated helper fault,
  fallback, and restart testing. Reports now retain exact physical voices;
  strict voice selection, runtime engine selection, and a Russian stress
  profile cover both RuTTS voices and registered Windows helpers.
- Added machine-readable helper soak reports with periodic health and
  cancellation probes plus working-set, private-byte, handle, thread, and CPU
  samples. Native Windows helper metrics are resolved conservatively from WSL;
  ambiguous processes are never sampled as evidence.
- Added opt-in server process-tree sampling to stress reports, including
  aggregate and per-executable steady-state resource summaries for the server
  and every helper descendant without storing executable paths.
- Added bounded repeated dispatch-time helper fault injection. Each cycle
  resolves the current dedicated child before acting, verifies exactly-once
  cancellation of outstanding work, proves the configured fallback, and then
  requires explicit recovery to the requested physical voice.
- Added tested diagnostic redaction that removes synthesis-text records,
  checkout and common user-home paths, process command lines, and Git filenames
  from default support bundles while retaining bounded lifecycle evidence.
- Bounded locked-input downloads and all release/source archive extraction by
  member count and uncompressed bytes; encrypted ZIP entries are rejected in
  addition to existing path, duplicate, link, checksum, and layout checks.
- Added an isolated, source-built RuTTS v6.3.3 companion with built-in male and
  female Russian voices, Unicode-to-KOI8-R conversion, rate, pitch, intonation,
  volume, bounded PCM, and cooperative cancellation handling. RuLex is not
  included.
- Added locked RuTTS source preparation, portable companion staging,
  deterministic binary and corresponding-source artifacts, provenance,
  licensing, relocation checks, and real helper-session acceptance tooling for
  a six-target Linux, macOS, and Windows release matrix.

### Fixed

- Normalized speech rates now use measured, monotonic per-engine curves for
  WinRT, eSpeak NG, Piper, RHVoice, Flite, RuTTS, and DECtalk, anchored to the
  established Eloquence English rate and a same-language Russian audit for
  RuTTS. Native limits remain explicit saturation points instead of causing
  unrelated midpoint speeds or WinRT's former sharp transition above `0.5`.
- Diagnostic actions now select an explicitly requested engine exactly instead
  of silently measuring a fallback. `--check` and `--dump-wav` also honor the
  CLI voice, rate, pitch, volume, and Piper-model settings.  Windows Eloquence
  and DECtalk helpers are accepted as initial server preferences and direct
  diagnostic engines.
- eSpeak voice discovery can now reuse a bounded, schema-checked inventory
  cache for verified content-addressed data, while mismatched, corrupt, custom,
  and system data safely fall back to native discovery.
- Optional helper engines now initialize concurrently with built-in discovery,
  preserving a complete deterministic first inventory without accumulating
  every helper's process-start latency during a fresh server launch.
- Empty helper audio chunks and marker batches are now rejected so content-free
  frames cannot indefinitely refresh synthesis progress timeouts. Helpers omit
  those response types when a synthesis has no audio or markers.
- Helper response collection now rejects PCM chunks received after the marker
  stream begins, enforcing the documented synthesis response ordering.
- Incomplete multipart timeline timeout and input-closure paths now have
  explicit regression coverage for their owned failed terminal identity.
- DECtalk cancellation received after request acceptance but before native
  dispatch now prevents synthesis instead of allowing the cancelled request to
  reach the native runtime.
- Flite companion development builds now link on Windows GNU targets despite
  the pinned runtime's Microsoft-style external inline declarations.

### Documentation

- Documented RHVoice installation/runtime discovery and the Flite SLT
  companion's installation, source build, optional voice files, platform
  acceptance, verification, licensing, and removal.
- Documented RuTTS installation, source builds, built-in voices, KOI8-R
  repertoire routing, manual stress notation, verification, licensing, RuLex
  exclusion, and removal.
- Recorded bounded Windows x64 GNU acceptance for both RuTTS voices, including
  exact server routing, cancellation, resource samples, mixed queues, hard
  stops, repeated helper death, eSpeak fallback, and explicit recovery.

## [1.5.1] - 2026-09-01

### Changed

- Moved the GPL-2.0-or-later Eloquence and DECtalk capture-helper source,
  shared protocol host, build targets, and source-contract tests into Omnivox.
  Emacsvox continues to own the pinned WSL bundle build and deployment.

### Fixed

- Eloquence and DECtalk helpers now negotiate and report missing, malformed,
  wrong-architecture, or incomplete user-supplied runtimes as `not_available`
  instead of exiting before the helper protocol starts. Native DLL loading is
  restricted to validated absolute x86 images, required exports, the selected
  DLL directory, and System32.
- Draft Piper verification now installs the complete generic release payload
  beside the companion, so missing or corrupt model tests exercise the packaged
  eSpeak fallback rather than a build-tree data path. Recovery publication now
  also evaluates after its intentionally skipped build ancestors.

### Documentation

- Documented the complete Eloquence and DECtalk runtime requirements and
  installation paths, including the durable DECtalk release and a separately
  labelled newer development build without treating it as a distributable or
  default-pinned runtime.

## [1.5.0] - 2026-08-31

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
- Locked a trained-from-scratch English test model whose model card declares a
  public-domain LibriVox dataset for CI-only synthesis, without adding or
  recommending the model in release artifacts.
- Added a deterministic Piper corresponding-source artifact containing the
  exact committed source, locked Cargo dependency sources, all native build
  inputs, and the corresponding ONNX Runtime source, with exhaustive manifest,
  Git-tree, input-lock, model-exclusion, and offline-Cargo verification.
- Gated tag releases on four native Piper companion builds, exact draft-asset
  synthesis, and corresponding-source verification.

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

[Unreleased]: https://github.com/bartbunting/omnivox/compare/v1.6.2...HEAD
[1.6.2]: https://github.com/bartbunting/omnivox/compare/v1.6.1...v1.6.2
[1.6.1]: https://github.com/bartbunting/omnivox/compare/v1.6.0...v1.6.1
[1.6.0]: https://github.com/bartbunting/omnivox/compare/v1.5.1...v1.6.0
[1.5.1]: https://github.com/bartbunting/omnivox/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/bartbunting/omnivox/compare/v1.4.1...v1.5.0
[1.4.1]: https://github.com/bartbunting/omnivox/compare/v1.3.0...v1.4.1
