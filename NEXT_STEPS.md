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

Logical voice/style state may also contain engine-independent post-synthesis
effects such as reverb or echo.  That state follows the same spans as voice and
ACSS changes, survives chunk and engine boundaries, and is applied by Omnivox
after a buffered engine has returned PCM.  Auditory icons and other timeline
actions may either interrupt the primary speech clock or overlay it without
pausing speech.

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
- requested text anchors for timeline actions when an engine can resolve them;
- PCM audio in the canonical Omnivox buffer format when available;
- marker and resolved-anchor metadata plus the actual engine/voice selected;
- cancellation, failure, and degradation information.

## Presentation Timeline and Effects Model

Omnivox needs a presentation model in addition to its engine model.  The
primary speech timeline advances as speech and inserted audio are consumed.
An overlaid sound starts at an anchored frame but does not advance that primary
clock.  Its tail may overlap following speech and still counts toward tracked
completion.  This distinction must be explicit rather than inferred from
which audio sink happens to receive a buffer.

The first common timeline actions are:

- **audio action**: a tone or decoded resource with an explicit `insert` or
  `overlay` mode, volume, and routing;
- **semantic event**: a zero-duration bounded identifier emitted only when its
  associated playback boundary is consumed;
- **effect-state change**: begin, replace, or end post-synthesis processing at
  a source-text boundary.

Legacy queued `a` remains overlay-compatible: it schedules an auditory icon at
the current presentation cursor and does not by itself pause speech.  A new
structured action can request `insert` when serial playback is wanted.
Immediate `p` sounds remain independent of the tracked speech presentation.
Capital and all-caps tones are overlays at their resolved text boundaries.

Timeline positions are separate from Aural Emacsvox lifecycle anchors such as
object, run, and transition.  Positions may name a span boundary or a bounded
UTF-8 source-text offset with before/after affinity.  The preprocessing stage
must therefore retain a source map when punctuation expansion, capitalization,
or split-caps handling changes the text sent to an engine.

One logical style can contain two classes of dimensions:

1. engine-rendered dimensions such as rate, pitch, stress, and richness; and
2. Omnivox-rendered dimensions such as reverb, echo, filtering, gain envelopes,
   and spatial processing.

The compiler snapshots both classes on every speech span.  Only supported
engine dimensions are sent with the synthesis request.  The post-synthesis
state is retained until the engine returns PCM, then applied to the matching
audio frames.  Active effect state continues across text chunks, physical
voice changes, and engine changes until an explicit state change ends it.
Audio icons remain dry unless their action explicitly selects an effect bus.

The timeline transform owns source-to-output frame mapping.  Inserted audio
shifts later markers and events; overlays do not.  Duration-preserving effects
retain marker positions.  Reverb and echo tails extend playback completion but
do not duplicate word markers or semantic events: those remain attached to the
primary dry-speech boundary.  More complex repeated or non-linear marker
semantics require a later explicit protocol rather than being guessed.

Buffered PCM engines receive the full timeline and post-processing behavior.
An external-playback engine may omit unavailable effects or exact placement,
but must still speak and report the degradation.  Fallback must distinguish an
unavailable voice, an unresolved text anchor, and an unsupported post-synthesis
effect.

Marker support is graded rather than all-or-nothing.  A resolved action reports
one of `exact`, `word_boundary`, `span_boundary`, or `omitted`.  A buffered
engine without markers still provides ordinary speech, whole-span effects,
queue-boundary audio actions, cancellation, completion, and engine/voice
fallback.  For an action inside one synthesized span, Omnivox first splits at a
safe text boundary when that preserves the request, otherwise moves the action
to a declared span boundary or omits only the optional action.  It never uses
synthesis-time timers as a substitute for playback positions and never drops
speech because optional placement metadata is unavailable.

The current eSpeak backend advertises no markers even though its synthesis
callback receives native event records.  Capturing and validating its word,
sentence, and mark events is planned; until then it exercises the markerless
buffered-engine degradation path.

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

Engine-wide runtime failures persist across dispatches. The first unavailable
or failed synthesis call opens a circuit for five seconds; subsequent failed
recovery probes back off for 15, 30, and then at most 60 seconds. While a
circuit is open, resolution routes around that engine. After the cooldown,
exactly one synthesis request is allowed to probe it while other requests keep
using fallbacks. A successful probe restores normal routing; a failed probe
falls back for the same chunk and extends the cooldown. Stop cancellation must
neither begin a fallback attempt nor count a cancelled probe as another
failure. Inventory health and generation reflect these transitions.

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
in [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md). Every built-in backend now
implements mandatory self-description, and the active engine's snapshotted
descriptor is available through a structured inventory request. Logical-voice
registration is now available as an atomic, generation-safe whole-set
replacement. It retains unresolved definitions with diagnostics and supports
idempotent retries. The Emacsvox
adapter now negotiates capabilities, discovers inventory, normalizes its ACSS
definitions, and sends portable ordered selectors to both speech processes.
Machine-specific exact IDs stay optional; property selectors late-bind against
each server's inventory. Registered bindings now drive queued per-span routing;
semantic Emacs voice preferences resolve to their generated ACSS voice IDs.
Queued logical voices now re-resolve and retry after bounded runtime failures.
Their normalized rate, average pitch, and volume now apply when the selected
engine advertises those dimensions; unsupported values degrade away, and a
runtime fallback recomputes degradation against the replacement engine.
Pitch range, stress, and richness still require the structured synthesis result
work in Phase 4.

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

Status on 2026-08-06: the deterministic registry core owns engines by stable
ID, validates descriptor voice ownership and defaults atomically, snapshots
inventory, and generations real descriptor changes. Server discovery and
logical resolution now use this registry. Windows eagerly populates it with
WinRT and eSpeak when available, advertises the compatibility-selected preferred
engine, and retains the other engine for fallback. Emacsvox voice codes now
carry bounded logical IDs, and dispatched batches snapshot their definitions
and fallback policy. Following speech spans synthesize with the selected engine
and physical voice while retaining FIFO playback order; missing routes fall
back to the preferred legacy engine, and hard stops fan out to every registered
engine.
Immediate speech remains on the preferred legacy engine. Dispatched batches now
re-resolve against the worker's current health-adjusted inventory so a missing
voice can be excluded locally and a failed or unavailable engine can be
bypassed.
The identical failed chunk is re-resolved and retried with a four-attempt cap;
generation checks prevent a stop from starting a fallback attempt. Engine-wide
failures now open a persistent circuit with 5/15/30/60-second bounded cooldowns,
one recovery probe, automatic restoration after success, and health/generation
projection into inventory responses. Engines have an additive recovery
preparation hook; built-in in-process engines use its no-op default. Restarting
an invalidated helper during a recovery probe is implemented by the generic
helper engine and has been verified against the DECtalk helper.

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
- Add an engine-neutral presentation timeline with source/span anchors,
  insert and overlay audio actions, semantic events, and persistent
  post-synthesis effect state.
- Generalize `AudioEffect` to return a composable audio/timeline transformation
  before effects beyond silence trimming alter duration.
- Preserve the dry-source marker contract for echo and reverb while accounting
  for their audible tails in tracked completion.

Status on 2026-08-07: buffered queued audio now has one-shot playback tickets.
Natural mixer-source exhaustion completes a ticket; stop, backlog clearing, or
source teardown cancels it. This supplies the queued Emacsvox contract, while
externally playing engines and full marker propagation remain part of this
phase. `TtsEngine::synthesize` now accepts an owned structured request and
returns audio plus the realized engine/voice, validated markers, and omitted
ACSS dimensions. Exact routed voices must be realized exactly. Helper marker
frames now leave the process client and their frame offsets are rescaled with
audio during canonical conversion. All engine PCM is converted to Omnivox's
stereo 44.1 kHz format before entering the effects and playback pipeline; this
includes the helpers' native mono 11.025 kHz output. `SilenceTrimmer` now
reports removed frames, and structured synthesis markers are shifted and
clamped to the retained audio timeline before volume and channel processing.
This focused API preserves the existing `AudioEffect` contract while trimming
is the only duration-changing effect. A composable timeline-aware effect
contract remains the planned generalization if more effects change duration.
Tracked audio sources also accept opaque caller-defined frame cues. They emit
cues in stable frame order, including frame-zero and terminal-frame cues, and
drop all unreached cues on cancellation without coupling the audio crate to TTS
marker types. These cues follow source consumption, so device buffering may
make them lead acoustic output slightly. Capability-gated delivery of those
events is now implemented server-side with a separate marker dispatch,
versioned Base64-JSON start/marker records, bounded payloads, and a flush
barrier before the existing tracked terminal record. Emacsvox negotiation and
event binding now expose those events through an explicit marker-aware speech
API. WinRT now enables its word and sentence boundary metadata, reads the
resulting speech cue tracks directly from each buffered synthesis stream, and
maps cue timestamps plus inclusive UTF-16 input positions into bounded PCM
frame offsets and UTF-8 byte ranges. Missing tracks or malformed individual
cues degrade to fewer markers or less source metadata without failing speech.
A real Windows playback smoke test delivered sentence and word events before
tracked completion. Eloquence and DECtalk word markers plus DECtalk
phoneme/native-index markers now follow the same helper, resampling, and
playback-cue path. External playback and unifying the two audio buffer types
remain.

#### Planned Presentation-Timeline Slices

Each slice is independently tested, documented, and committed before the next
one changes the contract.

1. **Timeline vocabulary and renderer tests.** Define stable action IDs,
   source/span positions, `insert` and `overlay` modes, persistent effect state,
   and a piecewise source-to-output frame map.  Test stable same-frame ordering,
   overlay tails, insertion shifts, and cancellation before changing playback.
2. **Queue-boundary overlays.** Lower legacy `a` to an overlay at the current
   presentation cursor.  Keep speech as the primary clock, allow the icon to
   overlap following speech, retain independent sound volume and stereo
   routing, and include the complete icon tail in stop and tracked-completion
   behavior.  Add an explicit structured `insert` action instead of changing
   legacy `a` to serial playback.
3. **Requested engine anchors.** Extend synthesis requests with bounded opaque
   anchor IDs and UTF-8 positions.  Add a negotiated helper protocol version
   while retaining version 1.  Eloquence resolves exact anchors with reserved
   ECI indexes alongside its automatic word indexes; resampling and silence
   trimming carry them through the existing marker map.  Other engines
   advertise exact, word-boundary-only, or no anchor support.
4. **Capitalization tones.** Convert queued single-capital and all-caps handling
   into 440 Hz and 1300 Hz timeline overlays at requested anchors, matching the
   standalone Eloquence behavior without splitting an Eloquence synthesis.
   Move letter capitalization onto the same audio-action implementation and
   retain a documented fallback for engines without exact anchors.
5. **Anchored audio resources.** Preload, bound, decode, resample, route, and
   cache icons before rendering.  Support both insertion and sample-aligned
   overlay within a speech span.  Render with bounded look-ahead windows so an
   icon or effect tail may cross a synthesis-chunk boundary without requiring
   the entire dispatch to finish synthesis before first audio.
6. **Playback-bound semantic events.** Attach zero-duration action IDs to the
   common playback-cue path, emit them only when reached, discard unreached
   events on stop/replacement, and flush them before the terminal dispatch
   record.  Negotiate a new bounded event protocol; Emacsvox retains the richer
   Lisp meaning associated with each opaque ID.
7. **Persistent post-synthesis effects.** Split normalized style application
   into engine-rendered and Omnivox-rendered dimensions.  Carry effect state
   through preprocessing, chunking, routing, fallback, and voice changes, then
   apply it to resolved PCM regions with click-free boundary ramps.  Begin with
   duration-preserving gain, filtering, and spatial effects, followed by reverb
   and echo with explicit tail and marker semantics.
8. **Aural Emacsvox transport and degradation.** Negotiate a structured
   timeline capability and send positions separately from lifecycle anchors.
   Lower before/after actions to the legacy queue when possible.  Report exact,
   approximated, and omitted anchors/effects without dropping speech, and keep
   old Omnivox servers usable.

Completion criterion: one tracked mixed-engine dispatch can carry persistent
voice and effect state, overlay an auditory icon and capitalization tone at
resolved speech boundaries, insert serial audio when requested, emit a semantic
event only when reached, preserve marker order, and cancel all unreached audio,
tails, and events atomically.

### Phase 5: Complete the Emacsvox Protocol

- Add capability/version negotiation so Emacsvox enables extensions only when
  the server advertises them.
- Add structured engine/voice discovery, selection, and status diagnostics.
- Implement `emacsvox_tx GENERATION {BASE64}` with UTF-8 decoding, bounded
  payloads, atomic validation, generation coalescing, stale-frame rejection,
  and stop-barrier semantics.
- Implement tracked dispatch with exactly one `completed`, `cancelled`, or
  `failed` terminal result.
- Negotiate marker-aware playback, decode bounded events, and expose explicit
  marker and terminal callbacks without changing ordinary speech dispatch.
- Negotiate structured presentation timelines with explicit audio modes,
  source positions, persistent post-synthesis effect state, degradation
  reports, and playback-bound semantic event IDs.
- Add generation-safe runtime routing policy so a client can set the global
  preferred engine order independently of startup environment variables while
  retaining per-logical-voice selector order and the global fallback list.
  Implemented: policy generations atomically control preferred, fallback, and
  disabled engine lists; logical bindings re-resolve, dispatched work keeps its
  snapshot, legacy speech follows the global preference, inventory exposes
  circuit/cooldown/disable state, and clients can arm explicit recovery probes.
- Add a one-shot exact-route preview request that carries a physical selector,
  normalized style, effect state, and sample text without replacing persistent
  logical-voice registration or notification-process state. Exact and portable
  selector preview, playback completion, realized-route reporting, and ACSS
  degradation are implemented; post-synthesis effect preview follows the
  timeline/effects contract below.
- Make language commands functional and include language in synchronized state.
- Map Emacs logical voices and ACSS styles to Omnivox logical voice definitions,
  including ordered physical voice fallbacks.
- Retain graceful behavior with clients that only send the legacy protocol.

Status on 2026-08-07: capability negotiation, inventory discovery, atomic
logical-voice registration, portable selector customization, and legacy-client
fallback are implemented in Emacsvox. Registered logical voices now select the
engine and physical voice for queued spans. Capability-gated `emacsvox_tx`
delivery now provides bounded UTF-8 decoding, atomic whole-frame validation,
generation coalescing and stale rejection, and an external stop barrier while
preserving the legacy path for older servers. Capability-gated tracked dispatch
now emits exactly one terminal result after all queued speech, tone, silence,
and audio-icon sources complete, cancel, or fail. Emacsvox negotiates this
feature per live process and retains its unsupported-server error for older
Omnivox builds. The server now advertises `playback_marker_events_v1` and
accepts `emacsvox_marker_dispatch` without changing the established tracked
contract. Emacsvox now negotiates that capability per process, consumes and
validates bounded version 1 events, preserves dispatch ownership and event
order, and exposes marker and terminal callbacks through `tts-speak-marked`.
It selects the new command only for that explicit API and rejects unsupported
servers before submitting text. Omnivox now also implements and advertises the
independent runtime routing-policy and explicit engine-recovery-probe requests;
Emacsvox workbench integration follows in the paired UI slice. Functional
language commands remain.

Completion criterion: Emacsvox can assign DECtalk Paul to one logical voice and
an Eloquence voice to another, use both within a dispatch, and continue speaking
through a configured fallback when either engine or voice is unavailable.

#### Emacsvox Voice Workbench Integration

The live inventory and routing protocol feed one generic Emacsvox Voice
Workbench rather than an Omnivox-only customization screen.  Voice palettes
remain portable style/effect definitions.  Separate versioned routing profiles
hold global engine order and ordered selectors for logical voices.  The UI
presents both together while saving exact machine-local IDs only in an
explicit local scope.

Omnivox supplies the workbench with live, generation-stamped engine inventory,
health and degradation state, exact non-mutating preview, atomic logical-voice
registration, runtime preferred-engine order, and last realized route events.
Static standalone Eloquence and DECtalk adapters, SwiftMac discovery, and
markerless/free-form adapters use the same Emacsvox inventory interface with
truthful reduced capabilities.

Completion criterion: without editing Lisp or raw selector data, a user can
select an existing logical voice, audition installed physical voices, assign
and reorder exact or portable fallbacks, tune its supported style/effects,
choose global engine priority, inspect the realized route, and atomically save
or cancel the staged configuration.

### Phase 6: Upgrade WinRT as the Reference Buffered Engine

- Return complete voice and language descriptors.
- Enable WinRT word and sentence boundary metadata. (Implemented.)
- Convert WinRT markers to the common result model. (Implemented.)
- Exercise common resampling and trim adjustment with WinRT markers.
  (Implemented through common pipeline tests and a real Windows playback smoke.)
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
  clean reference for the helper protocol. (Word markers implemented.)
- Add negotiated requested-anchor support so Eloquence can map Omnivox
  timeline actions to exact captured PCM frames without taking over playback.
- Implement DECtalk second, including phoneme/index metadata where available.
  (Implemented.)
- Keep the existing standalone `windows-outloud` and `windows-dtk` servers as
  fallbacks until Omnivox reaches practical parity.

The version 1 transport, request/response types, PCM and marker framing,
bounds, cancellation terminal states, and recovery contract are defined in
[HELPER-PROTOCOL.md](HELPER-PROTOCOL.md). The generic helper-backed engine now
negotiates inventory, validates exact voice and PCM results, cancels active
synthesis, invalidates failed children, and reconnects for recovery probes.
The Emacsvox tree now builds a separate 32-bit Eloquence capture helper without
changing its existing WaveOut bridge. It exposes ECI voices `v1` through `v8`,
bounded 11.025 kHz mono PCM, normalized rate, responsive cancellation, and
truthful reduced capabilities. Windows Omnivox discovers that helper beside its
executable or through `OMNIVOX_ELOQUENCE_HELPER`, negotiates it through the
generic host, and adds it to the registry without changing the WinRT legacy
default. The same shared host now drives a DECtalk capture adapter with nine
named voices, bounded in-memory PCM, rate mapping, and native cancellation;
Windows discovery treats its startup and failure independently. The Windows
staging target packages both helper executables and copies an already-installed
DECtalk runtime without downloading or redistributing proprietary files.
Real WSL-to-Windows tests have verified mixed Eloquence/DECtalk dispatch,
missing-engine and missing-voice fallback, tracked playback completion, and
DECtalk helper crash recovery. The shared helper host now accepts bounded marker
results and validates their kinds, frame offsets, source ranges, and values.
The Eloquence adapter segments bounded Unicode words, inserts an ECI index at
each word start, records index callbacks against captured PCM frames, and emits
UTF-8 source ranges. Direct helper tests covered apostrophes and non-ASCII text;
a routed Windows playback smoke carried the rescaled word events through
Omnivox before tracked completion. The DECtalk adapter allocates the native
phoneme and index arrays, emits their utterance-relative sample positions with
numeric engine values, and deliberately omits unavailable source-text ranges.
A direct helper test captured phonemes plus an inserted index, and a routed
Windows playback smoke carried rescaled DECtalk phoneme events through Omnivox
before tracked completion. DECtalk word capture now inserts collision-avoiding
private indexes around bounded Unicode words, excludes balanced native command
and phonetic spans, preserves caller indexes, and maps callbacks back to UTF-8
source ranges. Direct tests covered Unicode, an existing colliding index, and a
command embedded in a word; a comparison with the uninstrumented helper
produced identical PCM. Routed playback delivered the rescaled DECtalk word
events before tracked completion. Richer native controls, truthful
Eloquence/DECtalk sentence boundaries, long-session measurements, and broader
malformed helper-output testing remain.

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
- Expand the common post-synthesis effect set after the Phase 4 timeline work,
  without compromising latency, marker accuracy, or effect-state continuity.
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
- Test inserted and overlaid icons, cross-chunk effect spans and tails,
  requested-anchor degradation, and cancellation of unreached semantic events.
- Run the Emacs 31 suite and real WSL-to-Windows tests for each Windows engine.
- Measure first-audio latency, memory growth, long-session stability, and helper
  process overhead.
- Add user-facing engine/voice diagnostics and troubleshooting documentation.
- Exercise the generic Emacsvox Voice Workbench against live Omnivox,
  standalone static inventories, changing health, missing voices, and
  divergent speaker/notification inventories.
- Reconcile README, STATUS, architecture, and protocol documentation at each
  release.

## Repository TODO Reconciliation

The following previously documented goals remain in this roadmap:

- Linux Speech Dispatcher backend;
- TCP/network mode;
- multi-device routing;
- the common timeline renderer and optional reverb, echo, and chorus effects;
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
5. **Rich protocol**: framing, tracked completion, languages, capabilities,
   markers, timeline actions, and persistent post-synthesis effects work end to
   end with Emacsvox.
6. **Project backlog**: the remaining platform, packaging, routing, chunking,
   effects, and network goals are completed or explicitly deferred with reasons.
