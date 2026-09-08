# Per-fallback tuning: synthesis and playback handoff

Status: High implementation in progress, 2026-09-09. Routing, effects, ticket and
observation handoffs are tested; full wire support is not implemented. Reviewed against Omnivox `0cbfa93` on
`voice-choice-tuning`; typed composition and registration are already tested.
[ADR 0011](adr/0011-per-fallback-voice-tuning.md) and the
[paired wire contract](per-fallback-voice-tuning.org) remain authoritative.
No new user choice, wire field, dependency, helper protocol, calibration or
process boundary is introduced by this review. Continue implementation at High.

Completed first portion: admission snapshots retain layered definitions and
their generation alongside the compatibility projection, with queue byte
accounting. Actual attempts compose fresh settings and retain the original
resolution/choice identity. One internal retry loop hands buffered results or
committed stream metadata over with the prepared attempt; legacy callers use
an adapter. The layered entry remains internal and is not advertised.

Validation: all 667 locked workspace tests pass, including five new routing
tests covering all four buffered/streaming fallback combinations, frozen
registrations, fresh default settings across chunks, duplicate-row versus
policy identity, invalid stream identity and no replay after commitment/output
failure. Pinned workspace Clippy, formatting and documentation checks pass.
These routing tests exercise fake engine requests and handoff. The subsequent
[Windows native audit](benchmarks/2026-09-09-windows-native-defaults.md) confirms
set/default/set restoration for all 17 advertised Windows voices using actual
native parameter queries (85 captured syntheses). No native reset change is
needed for those tested runtimes. Playback observation ownership and the
remaining acceptance paths are still in the implementation sequence below.

Completed effects portion: dispatch processors now retain an explicit legacy
or layered owner. Actual-attempt handoff installs the selected effects and
rebuilds speech-bus action resources from immutable source data. Same-owner
windows retain state; an owner change flushes the previous bounded tail using
its old placement and selects a clean processor. Rejected prepared output
cannot become dry speech or a later audible effect tail. Four new pipeline/DSP
tests cover continuity, duplicate physical targets, legacy boundaries, tail
placement, speech-bus resources and rejected buffered/progressive rendering.
All 671 workspace tests, workspace Clippy, formatting and documentation checks
pass. New wire admission and the complete layered pipeline entry are still
pending; the bundle remains unadvertised.

Completed ticket portion: marker and private progressive sources hand off their
completion ticket immediately, including before fallible initial-cue setup.
Both completion and speech-clock lists retain it once; successful `finish` is
no longer the registration point. PCM publication records acceptance using
`published_frames()` even when attachment fails. Three new tests cover a held
consumer and partial failure with/without markers, invalid initial cues with a
settled ticket and no emitted event, and a real send-before-attachment-failure
path with accepted PCM but no consumption. All 674 workspace tests, workspace
Clippy, formatting and documentation checks pass.

Completed observation portion: buffered and progressive prepared output allocate
source handles before queueing. Acceptance and first consumed frame are separate
facts, including when playback wins the producer acknowledgement race. Evidence
deduplicates the complete choice identity, retains at most 32 entries and tracks
the last started choice independently. Empty or wholly trimmed output creates no
observation. Seven additional tests cover these rules, repeated identities,
truncation, and cancellation or partial failure behind a held consumer. All 681
workspace tests, workspace Clippy, formatting and documentation checks pass.
Terminal serialization and new wire admission remain next; production legacy
requests do not attach the new collector and the bundle remains unadvertised.

Completed preview codec portion: strict `preview_voice_v2` requests retain a
private full voice, sparse context, placement, original selection index and all
policy fields. Version-2 terminal encoding bounds acceptance entries and UTF-8
diagnostics while preserving status, frozen metadata and independent last-started
identity. Four fixture/adversarial tests cover round trips, missing/unknown/
duplicate fields, semantic validation and encoded-output truncation. All 685
workspace tests, workspace Clippy, formatting and documentation checks pass.
The live server still rejects this request pending its private execution path;
no capability is advertised by the codec alone.

Completed private execution portion: live `preview_voice_v2` admission validates
the complete draft and reserves terminal metadata space before queueing. Automatic
previews retain the private policy; individual auditions restrict only the
resolver projection and preserve the original full-row identity and index.
Both run the prepared-attempt pipeline and hand observations to the existing
ticket-waiting reporter. Empty output completes without evidence; pre-consumption
cancellation and committed failures retain their distinct status. Eight new
tests exercise the actual worker/reporter with simulated buffered and streaming
engines, strict duplicates, no substitute after failure, frozen disablement,
defaults above host rate one, queue accounting and unreportable identities before
synthesis. One additional codec test checks terminal-space reservation. All 694
workspace tests, workspace Clippy, formatting and documentation checks pass.
Timeline-4 ordinary speech, marker-3 receipts and client integration remain
pending, so the bundle remains unadvertised and no live runtime is replaced.

Completed timeline codec portion: an explicit mixed-span timeline-4 type and
bounded single-frame/reassembled-document codecs validate the new grammar,
positive generations, unique IDs and whole-document action references. Registry
validation rejects stale, missing or legacy targets while admitting unresolved
registered layered voices for later runtime fallback. Existing legacy style and
action validators are reused without flattening layered execution data. Six
tests cover the independent mixed fixture, malformed new fields, trustworthy
rejection identity, registry ownership, UTF-8 actions and separate transport
limits. All 700 workspace tests, workspace Clippy, formatting and documentation
checks pass. Reader assembly, queue admission and ordinary playback integration
still remain; existing timeline decoders and advertised capabilities are unchanged.

Completed marker codec portion: marker version 3 adds the strict typed
`voice_choice_applied` payload and accepts existing timeline event kinds. Receipts
are limited to 32 KiB decoded; every version-3 event is limited to the existing
512 KiB encoded line budget, including prefix and newline. Old event versions
retain their limits. Three new fixture/adversarial tests cover required identity,
duplicates, version isolation, escaping and both output limits. All 703 workspace
tests, workspace Clippy, formatting and documentation checks pass. First-frame
pair publication and pre-synthesis chunk preflight are still to be connected;
adding the codec does not advertise the bundle.

Completed receipt publication portion: a layered source uses one first-frame
cue and one reporter message for adjacent start/choice records, before other
frame-zero diagnostics. Both records retain their own bounded reservation.
The prepared-attempt router calls output preflight before native synthesis and
does not retry an encoding failure. A span ID belongs to the playback context,
independently of reusable style inputs. Four additional tests cover both output
modes, encoded-size preflight without consumed sequence numbers, paired capacity
release and rejection before any engine call. Existing held-consumer and
empty-output tests now also cover marker 3: cancelled unconsumed audio emits no
receipt; a consumed failed prefix does; empty/trimmed output emits no pair.
All 707 workspace tests, workspace Clippy, formatting and documentation checks
pass. Production timeline-4 admission must still supply the span context and
version-3 dispatch; the complete feature remains unadvertised.

Completed ordinary renderer portion: prepared timeline spans retain an explicit
legacy or layered style. The mixed executor uses fresh actual-attempt preparation
for both, emits choice receipts only for layered spans, and ends legacy effect
state at layered boundaries. Each legacy run begins neutral. Pure text encoding
and action-window checks run for every span before any engine call; actual-route
metadata is checked again on each attempt. Five new ordinary-playback tests cover
buffered/streaming fallback, contextual precedence/defaults, placement, neutral
legacy runs, immutable registry ownership, unresolved voices and atomic rejection
of invalid later spans/windows. All 712 workspace tests, workspace Clippy,
formatting and documentation checks pass. The ordinary entry remains internal
until reader/queue admission and multipart assembly are connected; old timeline
versions retain their existing adapter path and the bundle remains unadvertised.

Completed reader/queue portion: the existing command admits explicit old or
mixed documents without flattening. Version-4 registry, action-window and text
checks precede reader coalescing or cancellation leases. Multipart assembly
requires matching protocol versions throughout and in the decoded document;
partial frames cannot enter synthesis. Version-3 and version-4 replacement keys
remain separate in both reader selection and the bounded queue. Four additional
acceptance tests cover version changes, incomplete/replayed assemblies, invalid
later work preserving active speech, replacement domains, and complete reader,
queue, worker, consumed-choice and terminal reporting for both output modes.
All 716 workspace tests, workspace Clippy, formatting and documentation checks
pass. Both-lane/reconnect acceptance remains before capability advertisement;
no runtime has been installed and the Emacsvox client/editor remains pending.

## Findings that determine the implementation

- [Routing](../omnivox-cli/src/routing.rs) discards the resolver's reason/index
  when constructing `LogicalRoute`. A physical engine/voice pair cannot recover
  the selected row, especially for duplicate selectors or policy fallback.
- [The pipeline](../omnivox-cli/src/pipeline.rs) constructs
  `ProgressiveChunkSink` before repertoire/health/failure rerouting. Its
  `routed_effects` field holds one shared style, adapted using stream metadata.
  That is insufficient to select an individual patch after a retry.
- `RoutedAttemptStreamSink` already buffers start metadata and anchors until
  commitment. It is the appropriate boundary for handing over an attempt's
  style and identity together. Do not publish those during initial resolution.
- `ProgressiveChunkSink::finish` currently records its ticket only after a
  successful finish. Error and cancellation paths can therefore lose the
  completion barrier for audio already accepted by a source.
- [The progressive producer](../omnivox-audio/src/output.rs) exposes
  `published_frames()`. A non-empty send may succeed before source attachment
  fails, so a failed `push_audio` result alone does not establish zero acceptance.
- Existing marker callbacks are frame-boundary callbacks. A zero-frame terminal
  can reach boundary zero. New voice observations must be attached only to a
  source receiving non-empty rendered PCM, never to metadata or a cue-only
  completion. Playback start means server source consumption, not acoustic onset.

## Immutable inputs and one prepared attempt

At complete-request admission, retain the authoritative mixed registry and its
generation, effective fallback policy, effective disablement, original host
rate and independent output controls. Keep a compatibility projection for
legacy consumers. The worker may refresh inventory/health while those inputs
remain fixed. The dispatch/connection owns this snapshot; live registration is
not a lookup source for queued chunks.

Represent each admitted span's input as an explicit legacy or layered variant.
The layered variant references its admitted definition and owns sparse context
and placement. Do not put a mutable "current context" on the routing snapshot:
adjacent spans can name the same voice with different contextual rules.

An `Arc` may share immutable registry data between spans and requests. Account
for the retained definitions, choice/patch storage, compatibility projection
and span data in queue admission; conservatively charging shared storage per
request is acceptable. Include source observation handles in active playback
bounds. A 32-entry visible evidence list does not by itself bound all queued
source metadata, and cloning snapshots must not bypass the existing byte limit.

Use a CLI-owned immutable `PreparedVoiceAttempt` (name illustrative) containing:

- admitted registry/private-preview identity and original selected row index;
- resolver reason, stable choice ID or null, and selected physical identity;
- complete fresh `TtsSettings` and capability-adapted normalized ACSS;
- complete capability-adapted post-synthesis style and both degradation lists.

Build it after each actual route selection, including repertoire routing and
every permitted health/failure retry. Use one selected adapter capability
snapshot for that attempt. Compose shared, selected choice and sparse context,
then placement, then capability adaptation. Policy fallback has no choice patch,
even if the physical target appears in a stored row. The compatibility
projection is never an input for reconstructing a layered patch.

For layered requests, start native settings with the selected physical voice,
captured host rate, neutral native pitch multiplier 1 and voice volume 1; then
apply the composed ACSS. Preserve independent global volume and lane output
processing. Do not use a previously styled `TtsState.pitch_multiplier` as the
default. Relative rate is calculated once from the original admitted host rate;
zero/default/absence preserves rates above 1 as specified in the wire contract.
No numeric "native default" is invented for an omitted extended ACSS field.

The adapter boundary must also honor omitted fields after a customized request.
DECtalk/Eloquence currently reselect a voice and send optional extended controls;
the presence of a voice-selection prefix alone is not proof that all persistent
native parameters reset. Add same-voice set/default/set sequence tests at the
adapter/native-command boundary. Any necessary reset stays within the existing
adapter and helper message. Stop for review if this cannot be satisfied without
a protocol, calibration or process-boundary change. Do not advertise unsupported
default semantics based only on the pure composer's `None` values.

## Transfer ownership at commitment

Keep `TtsEngine`, `SynthesisRequest`, `SynthesisStreamStart` and helper envelopes
unchanged. Add a CLI-local routed sink interface with an operation equivalent
to `start_attempt(prepared_attempt, validated_start)`, followed by the existing
audio/marker operations. `RoutedAttemptStreamSink` remains the engine-facing
`SynthesisStreamSink` adapter.

The attempt adapter retains the prepared attempt, start metadata and bounded
marker/anchor preamble. It validates reported engine/actual voice against the
exact physical request, including missing or mismatching metadata. At the
existing commit boundary, it passes the attempt and start together to the
pipeline before forwarding its buffered preamble and PCM. The layered sink
uses that attempt's effects directly; it does not rediscover them by engine ID.

A buffered success similarly returns the prepared attempt alongside the
validated `SynthesisResult`. A streaming-to-buffered retry therefore supplies
the same complete handoff. For layered requests, choose buffered/progressive
execution from each actual attempt's capabilities and anchor support, including
a buffered primary that fails and reaches a progressive fallback. Keep old
request behavior behind the legacy variant and existing adapters.

Discard every uncommitted attempt's pending start, markers, style and identity
on retry. Preserve the four-attempt cap and existing engine-versus-voice failure
classification. Preserve the conservative no-replay gate: once start/preamble
has been committed to an output consumer, an output error terminates the chunk;
do not loosen that rule because trimming or buffering has not produced audible
frames. No retry follows committed PCM, an output failure or transport failure.
Neither gate is evidence of consumed audio.

## Effects belong to the committed route

Do not initialize or mutate DSP state using a predicted or failed uncommitted
choice. A prepared attempt contains values, not a processor borrowed from a
previous route. Install its effect owner when the sink receives that attempt.

Track an effect owner within the dispatch: legacy run, or layered registry
generation/logical voice/choice/physical voice. A legacy run has a new identity
after a layered boundary. Same-owner chunks retain the existing stateful DSP
and smooth parameter transitions; this avoids restarting effects at native
window or ordinary text-chunk boundaries.

When ownership changes, finish the previous committed owner's bounded tail
using its existing placement/barriers, then install a clean processor before
the next owner's PCM. Do not apply old filter or delay state to a new choice,
even when both rows resolve to the same physical voice. Pending failed attempts
have no tail. Context/default operations still set fresh target parameters;
they do not recover values from DSP history. Legacy retain/replace/end semantics
remain confined to consecutive legacy spans.

Keep capability omissions distinct from processing failures. The new path must
not silently queue dry or partly processed audio after an effect/render error
and then claim the complete prepared style was applied. Mark output failure,
retain evidence for any earlier accepted windows, and do not retry. Validate
resources and frame/encoding constraints before output wherever possible.

## Playback observation and terminal ownership

Use one preallocated observation handle per committed playback source. It owns
an immutable audio-choice identity and a shared started flag. Construct the
handle before queueing so the null backend may consume its first frame before
the producer receives the queue acknowledgement without losing the observation.
The handle contains no reference to the mutable route or live registry.

The producer owns the ordered, distinct accepted-audio list. On a successful
non-empty buffered enqueue, record the handle. For progressive output, inspect
`published_frames()` after a push even when it returns an error; any increase
establishes acceptance. Centralize this accounting so failure, cancellation,
`finish` and early return paths cannot bypass it. Check before consuming the
producer in `finish`. Zero/trimming-only output does not enter the list.

Keep at most 32 distinct identities, preserving first-acceptance order. Reuse
a retained identity's started flag for later chunks with that same identity;
do not join merely by physical voice. Count/encoding truncation sets the
explicit flag. The source's first-frame callback sets its started flag and
updates a separately owned last-started identity even if the list is full.

The last-started slot can be a request-local short mutex holding an `Arc` to
already allocated identity. Callback work is fixed-size flag/pointer updates
and sending already reserved event records: no formatting, resource lookup,
capacity wait or stdout I/O. Perform deduplication, list growth, encoding and
truncation on producer/reporter threads. A last-started snapshot is read only
after playback barriers settle, so serialization never holds a lock needed by
the audio callback. No global evidence collector or extra reporter thread is
needed.

Retain each progressive ticket as soon as the playback source is created,
before publishing its initial cues or first window. The marker queue helper
must offer that ownership handoff before any later fallible setup operation.
Register once in both the request's completion list and its existing speech
presentation clock; remove the success-only registration in `finish`. Every
error path drops/closes its producer so an unattached or incomplete source can
settle its ticket. Preserve overlay ordering and test the initial-cue failure
path, not just failures after `push_audio`.

The tracked playback reporter waits for all these tickets, then snapshots the
bounded evidence and constructs the one terminal response. A synthesis failure
remains failed even if its accepted prefix finishes playing; a cancellation
does not erase already consumed evidence. Before the terminal is sent, every
source is finished/cancelled and no later callback may change its last-started
summary. Queue retirement before synthesis returns empty observations once.

## Frame callbacks and wire events

For timeline 4, prepare the existing `utterance_started` and the new
`voice_choice_applied` as adjacent reserved records at the same first-frame
cue, before any other frame-zero marker/diagnostic. Reserve adjacent sequence
numbers; emit in that order. The observer update occurs in the same callback.
The receipt names the original span, admitted registry generation and prepared
attempt. Do not infer consumption from terminal synthesis or ticket creation.

Create this cue only when a non-empty rendered window is ready to be published.
Do not complete a newly created observation source with only its initial cue:
drop it on failure before first PCM. Empty buffered results and wholly trimmed
streams create no voice observation. Leading presentation actions retain the
existing utterance/source timing; the receipt does not assert acoustic onset
or that every source frame contains voice audio. Existing marker versions and
timing for legacy requests remain unchanged.

Private preview v2 uses the same first-frame observer through an internal cue,
without emitting public timeline markers or replacing the registry. Its
accepted-audio list and last-started slot survive synthesis return until tracked
playback finishes. Strict individual audition retains the original row index:
if a resolver uses a one-row private view, carry an explicit index mapping and
restore the full-chain reason/index before composition and evidence. Never
remap by physical identity or enable global fallback for that audition.

Preflight the 32 KiB decoded receipt and 512 KiB encoded marker-line bounds,
including the existing utterance text record, before synthesis/commitment as
required by the wire contract. Reserve both initial records together and
retain existing reporter backpressure and terminal ordering. Encoding failure
cannot drop just the receipt and continue claiming full support. Preview
serialization retains correlation, status, last-started and the truncation flag
while bounding the accepted list and diagnostic text.

## Acceptance tests and implementation sequence

These are required tests to implement, not claims about current playback code.
Use the [independent composition examples](protocol-fixtures/voice-choice-tuning.json)
as value oracles and extend the existing fake engines, held-consumer audio
fixtures and marker reporter tests. Avoid wall-clock sleeps as race assertions.

| Boundary | Required observation |
| --- | --- |
| Preferred unavailable or fails before PCM | Fallback native settings, DSP and receipt all use its original row; no primary observation. |
| Buffered/progressive combinations | All four primary/fallback mode combinations use one prepared-attempt contract; anchor requirements still force buffering where needed. |
| Same engine, different voices | Voice-local failure preserves the engine's other eligible voice and applies its patch. |
| Duplicate selectors and strict second-row audition | Second row's ID/index/patch survives; policy resolution of that physical voice still has null choice ID. |
| Changed registry/rate after admission | Queued spans use the admitted definition, context and host rate. |
| Native set/default/set | Settings and adapter calls clear pitch, extended ACSS, relative rate and effects without changing global volume/lane controls. |
| Metadata/anchors then engine failure | Retry receives a fresh sink; failed preamble never reaches playback. |
| Accepted PCM, consumer held, then synthesis failure | No retry; ticket survives; terminal waits; late first-frame consumption is reflected before the failed response. |
| Accepted PCM cancelled before first frame | Accepted list can be non-empty; started is false and last-started is null; no receipt. |
| Output attachment fails after successful send | `published_frames` proves acceptance; no retry or engine quarantine; ticket settles and response completes once. |
| Null consumer starts before enqueue returns | Final accepted entry has started true and last-started matches; producer bookkeeping order cannot lose it. |
| Empty output or all audio trimmed | No voice observation or receipt from frame-zero terminal cues. |
| More than 32 distinct identities | List is bounded/truncated; a later consumed identity still becomes last-started; an unconsumed attempt cannot overwrite it. |
| DSP owner changes and mixed legacy runs | No filter/delay contamination; prior committed tails keep their owner; same-owner windows retain continuity. |
| Initial-cue setup/finish/output errors | Every created source retains its ticket and retires; no terminal can race a surviving source callback. |
| Frame-zero events and output bounds | Start then choice receipt precede other events; UTF-8/escaping bounds checked before commitment; terminal stays behind emitted markers. |
| Legacy operations and both lanes/reconnect | Old payloads keep old semantics; per-lane snapshots and late-event rejection remain independent; no replay. |

Implement and commit in this order:

1. Attempt preparation and CLI handoff, with immutable snapshots, fresh native
   settings, per-attempt execution mode, route-owned DSP and adapter default
   tests. Test without advertising or accepting an incomplete client bundle.
2. Ticket/acceptance/first-frame ownership and bounded preview observations,
   using deterministic partial-failure, cancellation and callback-order tests.
3. Timeline-4 decoding/admission/multipart/remote framing, preview-v2 selection
   and marker-3 serialization; connect the tested handoff to ordinary playback.
4. Cross-layer admission, synthesis, consumption, both-lane and old-protocol
   acceptance tests. Only then advertise the complete bundle and proceed with
   the Emacsvox client/editor slices.

Run pinned-toolchain locked workspace tests, Clippy, formatting and documentation
checks for each implementation commit. The present review changes only notes;
it does not warrant a native rebuild, deployment or new release minimum.
