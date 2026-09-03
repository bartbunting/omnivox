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

1. Extend the correlated Emacs submission, protocol admission, synthesis, and
   mixer-source telemetry to first audible device output where a platform
   exposes a truthful measurement callback.
2. Maintain and extend `tools/benchmark_server.py` across real platforms for
   character, word, ordinary line, dense-action timeline, multipart timeline,
   and rapid keyed replacement workloads. Preserve raw samples and compare
   p50, p95, and p99 rather than averages alone. The initial
   [Windows x64 development baseline](../benchmarks/2026-09-01-windows-x64-c9458361eb57b94a.md)
   records all six workloads for WinRT, eSpeak NG, RHVoice, Flite, DECtalk,
   and Eloquence. The later
   [null-output pre-optimization baseline](../benchmarks/2026-09-03-windows-x64-null-f7204ac69b6010f1.md)
   adds RuTTS and TGSpeechBox with exact representative voices, a seeded order,
   and no audible playback; other platforms and repeat runs remain outstanding.
   The later anchored-streaming follow-up shows that Eloquence and DECtalk
   dense timelines no longer add a whole-result wait. Profile and reduce the
   continuous-sinc startup cost only with matched audible-quality and waveform
   evidence; do not restore per-chunk linear upsampling.
3. Maintain `tools/stress_server.py` across real engines and platforms for
   interleaved replacement keys, ordered and urgent traffic, hard stops,
   queued/buffered audio, late completion, and helper restart. Keep verifying
   that stale markers, semantic callbacks, and duplicate or late terminal
   history cannot escape; add physical-output observation where available.
4. Measure long-session memory, decoded-resource cache behavior, helper working
   sets, and quarantined native-call capacity on real platforms. The helper
   soak tool now records working-set/private-byte, handle, thread, and CPU
   samples on POSIX and native Windows helpers; multi-engine evidence and
   explicit release thresholds remain outstanding. Server stress can also
   group the root and all helper descendants by executable, separating startup
   growth from steady-state growth.
5. Keep malformed-input, queue-saturation, multipart timeout, and partial-write
   tests aligned with every protocol change.

## Priority 2: engine hardening

1. Expand real Windows repetition, cancellation, crash, and recovery testing
   for WinRT, Eloquence, and DECtalk, including helper working-set measurement.
2. Resolve the licensing, corresponding-source, and publishing-integration
   gates in the [Piper release plan](PIPER-RELEASE.md). Native dependency,
   packaging, relocation, and real-synthesis acceptance now pass on Linux x64,
   Windows x64, and macOS ARM64/x64. Keep this a reproducible optional-helper
   release problem, separate from engine-registry policy; it is not yet a
   published component.
3. Improve macOS marker and cancellation coverage without overstating what
   AVSpeechSynthesizer exposes.
4. Extend RHVoice live-runtime acceptance beyond Linux x64 and Windows x64,
   prioritizing Linux ARM64 where upstream runtime support is available. Keep
   macOS and Windows ARM64 labelled compile-only until compatible native
   runtimes pass discovery, synthesis, marker, cancellation, and shutdown
   acceptance.
5. Verify logical-language and text-repertoire routing against live
   multilingual voices on every supported engine.
6. Complete RuTTS native acceptance on Linux ARM64, macOS Intel/Apple Silicon,
   and Windows x64/ARM64 MSVC. Windows x64 GNU development acceptance now
   covers both voices, exact routing, cancellation latency, helper resources,
   hard stops, repeated helper death, fallback, and recovery. Continue with
   cold onset, high-rate intelligibility, and multi-hour helper memory evidence.
   Evaluate RuLex later as its own licensing, provenance, database, and
   cross-platform decision rather than silently adding it to the companion.
7. Keep eSpeak NG as the reliable Unicode-capable final fallback and retain
   regression coverage for its exact native anchors and source-accurate UTF-8
   word/sentence mappings.

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

- **sherpa-onnx with Inflect Micro and Kitten Nano:** evaluate one optional,
  isolated sherpa-onnx adapter rather than model-specific integrations. Measure
  model load, first-audio and complete-synthesis latency, PCM callback cadence,
  cancellation and replacement without helper restart, working set, high-rate
  intelligibility, and the consequences of absent source-accurate word
  markers. Keep runtime and model assets separately auditable, with explicit
  per-model licence and provenance records.
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
