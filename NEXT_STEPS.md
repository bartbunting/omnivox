# Omnivox Roadmap

This document is the canonical plan for Omnivox development. It combines the
existing project backlog with the Emacsvox protocol work and the longer-term
multi-engine voice architecture.

## End Goal

An Emacs voice should be a logical definition that resolves to a speech engine
and a voice belonging to that engine. Different logical voices in the same
dispatch may use different engines while Omnivox preserves speech order and,
for buffered engines, owns audio processing and playback.

For example:

```text
source-code:
  dectalk / Paul
  -> eloquence / Reed
  -> winrt / David
  -> system default

annotation:
  eloquence / Eddy
  -> dectalk / Betty
  -> system default
```

If an engine, voice, or optional capability is unavailable, speech should
degrade predictably instead of being dropped or causing the server to fail.

## Voice and Engine Model

The model needs to distinguish these concepts:

- **Engine**: a synthesizer implementation such as `winrt`, `eloquence`,
  `dectalk`, `espeak`, `macos`, `piper`, or `speechd`.
- **Physical voice**: a voice owned by one engine. Its stable identity is the
  structured pair `(engine_id, voice_id)`.
- **Logical voice**: an Emacs/ACSS voice name with a preferred physical voice,
  ordered alternatives, and normalized style properties.
- **Voice selector**: either an exact engine and voice pair or a request to
  match properties such as language.

Engine and voice IDs must remain separate fields internally and in new
structured protocol messages. They must not depend on splitting an ambiguous
display string because native voice identifiers may themselves contain colons
or other separators.

The engine description should report at least:

- stable ID, display name, version, availability, and health;
- discovered voices and their stable IDs, names, and languages;
- supported ACSS dimensions such as rate, average pitch, pitch range, stress,
  richness, volume, and native style controls;
- whether synthesis returns PCM or owns playback externally;
- cancellation, streaming, and concurrent-request behavior;
- supported word, sentence, phoneme, and native index markers;
- language switching and audio format details.

The synthesis request/result contract should eventually contain:

- requested logical and physical voice information;
- normalized ACSS and language settings;
- PCM audio in the canonical Omnivox buffer format when available;
- marker metadata and the actual engine/voice selected;
- cancellation, failure, and degradation information.

## Fallback Contract

Voice resolution must be deterministic and testable:

1. Use the requested engine and exact voice when both are healthy and
   available.
2. Try the logical voice's explicitly configured alternatives in order.
3. If policy permits, use a compatible same-language voice on the requested
   engine.
4. Use the configured global default engine and voice.
5. Use a default voice from any healthy fallback engine, normally eSpeak.
6. If no engine can speak, keep the server alive and report a visible and, when
   applicable, tracked failure. Never silently discard the text.

Fallback also applies independently to capabilities. For example, an engine
without richness or word-marker support should still speak using the style and
metadata it can provide. Unsupported numeric settings are ignored or clamped
according to the engine descriptor.

Resolution must happen against current availability and be repeated after a
runtime `VoiceNotFound` or engine-unavailable failure. Retries must be bounded.
Diagnostics should expose both the requested and realized engine/voice without
injecting unexpected speech into the user's session.

Legacy `tts_set_voice` and `OMNIVOX_ENGINE` behavior must remain supported.
Legacy clients select the preferred/default engine; newer clients can select a
structured engine and voice per queued span.

## Roadmap

### Phase 1: Stabilize the Current Windows and Emacsvox Integration

- Fix `a` and `p` parsing for Tcl-quoted paths and WSL-to-Windows paths.
- Test queued auditory icons, immediate sounds, cancellation, ordering, cache
  behavior, and missing-file diagnostics.
- Verify main and notification speech processes use Omnivox consistently and
  retain the notifier lifecycle fix made in Emacsvox.
- Add interactive engine/voice discovery and selection to the Emacsvox adapter.
- Keep Windows cross-compilation, runtime DLL staging, and real WSL-to-Windows
  smoke tests reproducible.

Completion criterion: ordinary Emacsvox speech, auditory icons, notifications,
stop, and server switching work reliably with the current WinRT engine.

### Phase 2: Specify the Rich Engine and Logical Voice Contracts

- Define `EngineDescriptor`, `EngineCapabilities`, `VoiceDescriptor`, physical
  voice identity, logical voice definitions, and availability/health states.
- Specify the fallback algorithm above, including runtime failure behavior and
  diagnostics.
- Define normalized ACSS properties rather than embedding vendor control codes
  in the core model.
- Define a controlled way for engines to expose optional native extensions.
- Decide the compatibility encoding for legacy voice commands and the
  structured representation used by the new protocol.
- Write pure resolver and capability-degradation tests before refactoring the
  synthesis path.

This phase changes public internal contracts and should be reviewed before
implementation begins.

Status on 2026-08-06: the additive Rust model and pure resolver are implemented
without changing the legacy synthesis path. Tests cover exact and property-based
selectors, explicit alternatives, same-language fallback, global and engine
fallbacks, degraded and failed engines, environment-specific late binding,
fallback exhaustion, normalized ACSS clamping, and capability degradation. The
separate versioned Base64-JSON control envelope, bounds, capability negotiation,
structured errors, and compatibility boundary are implemented and documented
in [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md). Inventory and logical-voice
registration messages remain to be added.

### Phase 3: Add an Engine Registry and Per-Span Routing

- Replace the single engine chosen by `create_engine()` at startup with a
  registry of available engines.
- Support eager discovery for inexpensive native engines and lazy startup for
  proprietary or model-backed engines.
- Attach a logical/physical voice selector to each queued speech span so an
  inline voice change may also change engines.
- Resolve fallbacks before synthesis and re-resolve after bounded runtime
  failures.
- Preserve FIFO order across engine transitions and prevent stale synthesis
  results from entering playback.
- Track engine health and allow a failed helper process to restart without
  restarting Emacsvox.
- Preserve existing command-line and environment selection as compatibility
  policy.

Completion criterion: one dispatch can alternate between mock engines and
voices while preserving text, order, stop behavior, and deterministic fallback.

### Phase 4: Enrich Synthesis Results and the Audio Model

- Replace the minimal `TtsSettings -> AudioBuffer` interface with structured
  synthesis requests and results.
- Report the actual engine and voice used after fallback.
- Carry word, sentence, phoneme, and native index markers when supplied.
- Define truthful cancellation and completion semantics for synchronous,
  streaming, and externally playing engines.
- Unify `omnivox-tts::AudioBuffer` and `omnivox-audio::AudioBuffer` into one
  canonical type.
- Preserve or adjust marker timestamps through resampling, silence trimming,
  and other audio effects.

### Phase 5: Complete the Emacsvox Protocol

- Add capability/version negotiation so Emacsvox enables extensions only when
  the server advertises them.
- Add structured engine/voice discovery, selection, and status diagnostics.
- Implement `emacsvox_tx GENERATION {BASE64}` with UTF-8 decoding, bounded
  payloads, atomic validation, generation coalescing, stale-frame rejection,
  and stop-barrier semantics.
- Implement tracked dispatch with exactly one `completed`, `cancelled`, or
  `failed` terminal result.
- Make language commands functional and include language in synchronized state.
- Map Emacs logical voices and ACSS styles to Omnivox logical voice definitions,
  including ordered physical voice fallbacks.
- Retain graceful behavior with clients that only send the legacy protocol.

Completion criterion: Emacsvox can assign DECtalk Paul to one logical voice and
an Eloquence voice to another, use both within a dispatch, and continue speaking
through a configured fallback when either engine or voice is unavailable.

### Phase 6: Upgrade WinRT as the Reference Buffered Engine

- Return complete voice and language descriptors.
- Enable WinRT word and sentence boundary metadata.
- Convert WinRT markers to the common result model.
- Adjust marker positions after trimming and resampling.
- Improve voice, language, rate, pitch, and volume mapping.
- Advertise the limitation that a synchronous WinRT synthesis call cannot be
  interrupted internally even though playback and stale-result queuing can be
  stopped immediately.

### Phase 7: Add Eloquence and DECtalk Through a Reusable x86 Host

The proprietary Eloquence and DECtalk libraries are 32-bit while the Windows
Omnivox process is 64-bit. Use a versioned helper-process protocol rather than
loading those libraries into the Rust process.

- Reuse the existing Emacsvox C# bridge workers where practical.
- Add capture mode so helpers return or stream PCM instead of owning WaveOut
  playback.
- Define version negotiation, request IDs, audio format metadata, errors,
  cancellation, markers, bounds, timeouts, and crash recovery.
- Keep proprietary DLLs user-supplied and document redistribution constraints.
- Implement Eloquence first because its waveform and index callbacks provide a
  clean reference for the helper protocol.
- Implement DECtalk second, including phoneme/index metadata where available.
- Keep the existing standalone `windows-outloud` and `windows-dtk` servers as
  fallbacks until Omnivox reaches practical parity.

### Phase 8: Bring Other Engines Into the Same Model

- **eSpeak**: populate full descriptors and remain the reliable cross-platform
  final fallback.
- **macOS AVSpeechSynthesizer**: expose native voice/language capabilities and
  markers where available.
- **Piper**: stabilize the existing opt-in backend, revisit dependency and model
  packaging, and test supported cross-compilation targets.
- **Speech Dispatcher**: implement the planned Linux backend after a focused API
  spike. Speech Dispatcher normally owns audio playback rather than returning
  PCM, so advertise it as an external-playback engine with reduced centralized
  mixing, effects, and marker guarantees. Do not claim buffered-engine parity.

See [SPEECHD-PLAN.md](SPEECHD-PLAN.md) for the current Speech Dispatcher design;
it will need revision to use the common capability and completion contracts.

### Phase 9: Complete the Existing Omnivox Backlog

- Make text chunk size configurable if testing shows a useful need.
- Split intelligently at sentence/clause boundaries while retaining screen
  reader responsiveness.
- Benchmark synthesis overhead and add chunked/non-chunked integration tests.
- Add multi-device audio routing, including explicit speech and notification
  devices.
- Add TCP/network mode for the existing `-p` concept with authentication and
  exposure risks documented before enabling non-loopback listeners.
- Add optional effects such as reverb, echo, and chorus without compromising
  latency or marker accuracy.
- Complete language switching tables; the protocol work in Phase 5 supplies the
  underlying state model.
- Create a Homebrew formula/tap that installs the binary and Emacs module and
  prints integration instructions.
- Improve diagnostics for engine selection, fallback, chunking, voice
  resolution, and audio-device failures.

### Phase 10: Hardening and Release Readiness

- Add resolver matrices covering missing engines, missing voices, languages,
  unsupported capabilities, engine failures, and fallback exhaustion.
- Add parser/framing fuzz and malformed-input tests with strict payload bounds.
- Test mixed-engine ordering, cancellation, stale-result rejection, tracked
  completion, and helper restart behavior.
- Run the Emacs 31 suite and real WSL-to-Windows tests for each Windows engine.
- Measure first-audio latency, memory growth, long-session stability, and helper
  process overhead.
- Add user-facing engine/voice diagnostics and troubleshooting documentation.
- Reconcile README, STATUS, architecture, and protocol documentation at each
  release.

## Repository TODO Reconciliation

The following previously documented goals remain in this roadmap:

- Linux Speech Dispatcher backend;
- TCP/network mode;
- multi-device routing;
- optional reverb, echo, and chorus effects;
- language switching tables;
- smart/configurable text chunking, benchmarks, and integration tests;
- unifying the two `AudioBuffer` types;
- Homebrew packaging.

Two older documentation entries are stale:

- The parser has regression tests showing that text containing `;;` is
  preserved. If the audible symptom is reproduced, investigate preprocessing
  and the selected TTS engine rather than describing it as a confirmed parser
  defect.
- Piper is no longer merely an evaluated/rejected idea. An opt-in Piper backend
  exists; the remaining work is stabilization, packaging, and portability.

## Milestones

1. **Reliable baseline**: WinRT speech, auditory icons, notifications, and stop
   work from Emacsvox on Windows.
2. **Multi-engine core**: logical voices resolve to physical engine/voice pairs
   and mock/available engines can alternate within one dispatch.
3. **Graceful degradation**: removing a configured voice or engine still
   produces ordered speech through the documented fallback and exposes what was
   selected.
4. **Eloquence and DECtalk**: buffered x86 helpers make their voices first-class
   Omnivox choices.
5. **Rich protocol**: framing, tracked completion, languages, capabilities, and
   markers work end to end with Emacsvox.
6. **Project backlog**: the remaining platform, packaging, routing, chunking,
   effects, and network goals are completed or explicitly deferred with reasons.
