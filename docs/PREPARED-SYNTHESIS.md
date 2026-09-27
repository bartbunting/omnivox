# Prepared synthesis attempts and playback ownership

This maintained implementation reference describes the layered routing path in
[`routing.rs`](../omnivox-cli/src/routing.rs),
[`pipeline.rs`](../omnivox-cli/src/pipeline.rs) and
[`marker_events.rs`](../omnivox-cli/src/marker_events.rs).
[ADR 0006](adr/0006-voice-selection-and-customization.md) records the decision;
the [layered](per-fallback-voice-tuning.org) and
[native-parameter](engine-voice-parameters.md) contracts own exact wire semantics.
Current qualification belongs to [status](STATUS.md); historical checks remain
in the [retained routing report](benchmarks/2026-09-27-retained-routing-results.md).

## Immutable inputs and actual attempts

Admission retains the authoritative mixed registry/generation, private-preview
identity where applicable, effective policy/exclusions, original host rate and
independent output controls. Each span carries explicit legacy/layered inputs,
sparse context and placement. Later registry edits are not lookup sources for
queued work. Health/inventory may refresh without changing admitted definitions.
Queue accounting includes retained definitions, choices, patches and span data;
sharing an `Arc` does not bypass byte bounds.

The CLI's `PreparedVoiceAttempt` binds the actual resolver reason, original row
index, stable choice ID or policy-fallback identity, physical voice, fresh native
settings, adapted common controls, effects and degradation. Every permitted
retry prepares a new attempt after actual selection. Policy fallback has no
choice patch even when the physical voice also occurs in a stored row.

Compose shared, selected-choice and contextual settings before placement and
capability adaptation. Start layered native settings from the selected voice,
captured host rate and neutral pitch/voice-volume defaults, preserving independent
global volume and lane controls. Relative rate is computed once from the admitted
rate; zero/default/absence preserves rates above one. Missing extended controls
do not acquire invented numeric defaults. Qualified adapters reset persistent
native state; set/default/set regression tests verify that boundary.

## Transactional handoff

`RoutedAttemptStreamSink` retains the prepared attempt, validated stream-start
identity and bounded marker/anchor preamble. At commitment it hands the attempt
and start to the pipeline before forwarding PCM. Buffered success likewise
returns the attempt with its validated result. Each retry chooses buffered or
progressive execution from the actual engine's capability and anchor support;
all four primary/fallback combinations use the same ownership contract.

Uncommitted retry discards pending metadata, markers, settings and identity.
Preserve the four-attempt cap and engine-versus-voice failure classification.
Once output handoff is committed, an output error terminates the chunk even if
trimming or device buffering has not produced audible frames. Committed PCM
cannot be replayed, and output failures do not quarantine synthesis engines.
Commitment is distinct from source consumption.

## Effects follow the committed route

The pipeline installs effects from the prepared attempt, not a predicted route
or a lookup by engine ID. Its owner is a legacy run or the full layered
generation/logical/choice/physical identity. Same-owner windows retain stateful
DSP and smooth parameter transitions. Ownership change completes the prior
bounded tail with its existing placement/barriers and installs a clean processor.
Duplicate physical targets in different choices remain different owners; a
legacy run starts fresh after a layered boundary. Failed uncommitted attempts
have no tail.

Render errors cannot silently produce dry/partially processed speech labelled as
the full requested style. Preserve earlier acceptance evidence, terminate output
and do not retry. Validate resources and frame/encoding constraints before output
where possible; capability degradation and processing failure remain distinct.

## Accepted PCM, started sources and tickets

Allocate an immutable observation handle before queueing each committed source.
The null consumer may reach its first frame before enqueue acknowledgement.
Callbacks update fixed-size flags/pointers and send pre-reserved events; they
perform no formatting, resource lookup, capacity waits or stdout I/O.

Buffered nonempty enqueue records acceptance. Progressive paths inspect
`published_frames()` even after a push error: sending may succeed before source
attachment fails. Empty or wholly trimmed audio supplies no voice observation.
Retain at most 32 distinct complete identities in first-acceptance order with
explicit truncation, plus an independent last-started identity. Repeated identical
choices may share their started flag; physical voice alone is insufficient.

Retain each progressive completion ticket immediately when its source is created,
before fallible initial cues or first PCM. Register it once in the completion
list and speech presentation clock. Error paths close/drop the producer so
unattached or incomplete sources can settle. Final serialization waits for all
tickets and reached events, then snapshots evidence. A consumed failed prefix
remains observable; cancellation does not erase earlier consumed audio. Exactly
one terminal response follows, with no later callback able to change its summary.

## Frame evidence and private previews

Timeline 4 reserves adjacent `utterance_started` and `voice_choice_applied` records
at the same first-frame cue before other frame-zero diagnostics. The receipt binds
the original span, admitted generation and prepared choice. Native timeline 5
extends that evidence under its negotiated contract. Synthesis completion,
parameter preparation and ticket creation are not playback observations.

Create voice cues only for nonempty rendered PCM; a cue-only terminal cannot
invent a voice start. Leading actions retain their own timing. These events
describe source consumption, not acoustic onset or voice audio in every frame.
Preflight decoded receipt and encoded-line bounds before synthesis, reserve both
records together and preserve reporter backpressure/terminal ordering.

Private complete previews use the same executor and first-frame observer without
public timeline markers or registry mutation. Strict individual audition retains
the original row index/identity through its private resolver projection; it cannot
recover identity by physical voice or enable unrelated fallback. Bounded terminal
encoding preserves correlation, status, last-started identity and truncation.

## Regression obligations

Use the independent [composition fixtures](protocol-fixtures/voice-choice-tuning.json)
and deterministic fake engines/held consumers for races; elapsed sleeps alone are
not race assertions. Cover changed admission inputs, duplicate rows, policy
fallback, all buffered/progressive combinations, fresh adapter defaults,
rejected preambles, DSP owner transitions and legacy boundaries.

Exercise accepted-but-unconsumed failure, cancellation before first frame,
attachment failure after send, null consumption before acknowledgement, empty
output, more than 32 identities, cue/setup/finish errors and exact terminal
ordering. Preserve pre-synthesis encoding rejection, multipart/version isolation,
both workers, reconnect clearing and old-peer compatibility. Native reset,
listening and performance acceptance remain separate from deterministic fixtures.
