# Omnivox Roadmap

This is the current project backlog. It intentionally does not repeat shipped
architecture or a chronological implementation diary; see
[STATUS.md](../STATUS.md), [ARCHITECTURE.md](../ARCHITECTURE.md), and Git
history for
those records.

## Direction

Omnivox should remain a bounded, responsive multi-engine host for interactive
accessibility. Speech must degrade predictably when an optional engine, voice,
marker, effect, or resource is unavailable. Ordered and urgent output must not
be sacrificed to navigation replacement, and completion must describe actual
mixer-source lifecycle rather than command acceptance.

The structured identity model remains:

- an **engine** is a synthesizer implementation;
- a **physical voice** is the pair `(engine_id, voice_id)`;
- a **logical voice** is a portable Emacs style with ordered selectors;
- engine-rendered ACSS and Omnivox-rendered post-synthesis effects are distinct;
- exact machine-local IDs never replace portable fallback policy.

## Priority 1: latency and lifecycle evidence

1. Add end-to-end onset telemetry that correlates Emacs submission, protocol
   admission, synthesis, mixer-source consumption, and—where the platform
   permits measurement—first audible device output.
2. Maintain reproducible warm/cold latency benchmarks for character, word,
   ordinary line, dense-action timeline, multipart timeline, and rapid keyed
   replacement workloads. Report p50, p95, and p99 rather than averages alone.
3. Stress domain-scoped cancellation with interleaved replacement keys,
   ordered and urgent traffic, queued/buffered audio, late engine completion,
   and helper restart. Verify that stale audio, markers, semantic callbacks,
   and terminal history cannot escape.
4. Measure long-session memory, decoded-resource cache behavior, helper working
   sets, and quarantined native-call capacity on real platforms.
5. Keep malformed-input, queue-saturation, multipart timeout, and partial-write
   tests aligned with every protocol change.

## Priority 2: engine hardening

1. Expand real Windows repetition, cancellation, crash, and recovery testing
   for WinRT, Eloquence, and DECtalk, including helper working-set measurement.
2. Stabilize Piper dependency pinning and model packaging, cross-platform
   builds, cold start, cancellation/restart latency, and documented model
   ownership. Treat this as a reproducible optional-helper release problem,
   separate from the engine-registry policy, and solve it consistently for
   Windows, Linux, and macOS. Once distributable, a configured Piper helper can
   join the retained server registry without displacing each platform's
   built-in engines.
3. Improve macOS marker and cancellation coverage without overstating what
   AVSpeechSynthesizer exposes.
4. Verify logical-language and text-repertoire routing against live
   multilingual voices on every supported engine.
5. Keep eSpeak NG as the reliable Unicode-capable final fallback and extend
   native marker coverage only when mappings remain source-accurate.

## Priority 3: deployment and user diagnostics

1. Decide whether Linux ARM64 should join the Linux x64 GitHub artifact and
   runtime-test matrices, and evaluate a broader Linux ABI baseline than the
   current Ubuntu 24.04 build.
2. Add signing and provenance verification appropriate to Windows and macOS
   release artifacts.
3. Improve user-facing route, fallback, cancellation, and audio-device
   diagnostics while keeping full synthesis text opt-in and visibly sensitive.
4. Complete real-machine Voice Workbench apply/undo, migration, and divergent
   speaker/notification inventory coverage in the Emacsvox repository.
5. Reconcile README, status, protocol, and deployment documents as a release
   gate instead of storing completed phases in the roadmap.

## Explicit future proposals

These are not current features and require design or scope approval:

- **Multiple instances of one engine:** do not add a second Eloquence helper
  merely as a precaution. First collect long-session failure evidence for the
  persistent ECI owner-thread implementation. Revisit per-instance identity,
  health, retry, and duplicate-output rules only if failures remain frequent
  enough to justify the added state and resource cost.
- **Speech Dispatcher:** start from the capability and lifecycle contract in
  [SPEECHD-PLAN.md](SPEECHD-PLAN.md), then revise it for the current engine
  registry. External playback cannot claim buffered mixing/effects parity.
- **Multi-device audio:** define device ownership, fallback, restart, and
  notification separation before extending channel routing.
- **TCP/network mode:** require authentication, safe binding defaults, protocol
  exposure review, and explicit privacy documentation before implementation.
- **Additional effects:** new duration-changing or repeating effects must
  preserve marker semantics and truthful tracked completion.
- **Configurable chunking:** add a public control only if benchmarks show a
  useful trade-off beyond the current sentence/clause-aware hard limit.

## Release acceptance

A release candidate should satisfy all applicable locked checks and then pass
real-platform scenarios for:

- startup, voice discovery, route registration, preview, and clean shutdown;
- ordinary, replaceable, ordered, and urgent speech;
- hard stop and repeated keyed replacement during cancellable and
  uncancellable synthesis;
- mixed engines, missing voice/engine fallback, circuit recovery, and helper
  replacement;
- inserted/overlaid resources, effect-state continuity, marker/action ordering,
  and truthful terminal status;
- long input, multipart timelines, malformed records, saturation, and bounded
  resource failure;
- warm and cold onset distributions plus long-session resource stability.

Platform-dependent claims must name the engine, OS, toolchain, sample count,
clock, and instrumentation point. Mixer consumption is a useful proxy but must
not be labelled as first audible output.
