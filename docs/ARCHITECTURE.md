# Omnivox Architecture

## Runtime boundary

Omnivox is a buffered speech server. All currently registered server engines
return PCM to the host; Omnivox canonicalizes, processes, schedules, and plays
that audio. A future external-playback backend would have reduced timeline and
effects guarantees and must advertise that difference explicitly.

The legacy Emacspeak line protocol and the Emacsvox control/timeline protocols
share one bounded admission path. Configuration, inventory, and presentation
frames are parsed on the protocol side; engine synthesis never blocks receipt
of stop or newer commands.

## Workspace structure

```text
omnivox-core/          legacy commands, queue/state types, pure timeline model
omnivox-audio/         canonical buffer, effects, resources, renderer, playback
omnivox-tts/           engine contracts/backends, routing, protocols, helpers
omnivox-cli/           executable, admission, work queue, routing and pipeline
omnivox-piper-helper/  optional isolated Piper executable and protocol tests
omnivox-piper-sys/     optional maintained libpiper C API build and bindings
windows-helpers/       32-bit Eloquence/DECtalk capture processes and host
third-party/           separately licensed, provenance-recorded native source
elisp/                 standalone upstream-Emacspeak compatibility adapter
```

The main server is `omnivox-cli`. Omnivox-specific data contracts live in
`omnivox-tts`; the audio crate owns the single canonical `AudioBuffer`. The
core crate remains independent of any one engine. The C# Windows helpers are
separate executables and retain their GPL-2.0-or-later source license.

## End-to-end flow

```text
Emacs writes newline-delimited commands
                |
                v
bounded stdin reader (512 KiB/line, 32-line handoff)
                |
                v
protocol loop
  - parse legacy/control/timeline records
  - negotiate capabilities and validate bounded payloads
  - atomically assemble multipart timelines
  - advance hard-stop cancellation; prepare keyed cancellation leases
  - coalesce matching replaceable input only
                |
                v
bounded nonblocking synthesis work queue
  - 32 waiting requests / 32 MiB estimated owned payload
  - preserve ordered and urgent work
  - atomically replace matching queued domains
                |
                v
single synthesis worker
  - snapshot live engine health and routing policy
  - preprocess and sentence/clause-aware chunk text
  - resolve logical engine/voice route and bounded fallbacks
  - synthesize with cancellation checks
  - canonicalize PCM and markers
  - render effects and timeline actions in bounded windows
                |
                v
AudioControl / rodio sinks
  - speech, tone and sound streams
  - per-source and stream-wide cancellation
  - playback tickets and frame cues
                |
                +--> bounded marker reporter --> flushed stdout events
                +--> bounded playback reporter --> terminal status after source end
```

On macOS the protocol loop runs off the main thread because
AVSpeechSynthesizer requires the main NSRunLoop. On other platforms the
protocol loop may occupy the main thread. In every case a dedicated bounded
stdin reader prevents a producer from creating unbounded line memory, and the
synthesis worker is distinct from protocol admission.

Rodio consumes audio asynchronously. Marker events describe mixer-source frame
consumption, not guaranteed first output from the physical device. The tracked
playback reporter waits for every ticket owned by a request and flushes reached
marker events before writing its terminal record. The marker reporter bounds
outstanding marker-event work to 8,192 records and 16 MiB of serialized records
while preserving reached events; the tracked-completion handoff holds at most
32 pending reports.

## Admission and atomicity

Every protocol line is limited to 512 KiB, excluding newline and an optional
carriage return. An oversized or invalid-UTF-8 record is drained and rejected;
the following line remains parseable. The reader-to-protocol handoff holds at
most 32 complete lines.

An unframed legacy transaction holds at most 4,096 items and 16 MiB of
text/resource payload. Crossing a limit poisons and clears that pending
transaction so dispatch cannot synthesize a partial prefix. Stop and reset
clear the rejection.

Control and timeline envelopes impose their own decoded bounds. A version 3
timeline may use up to 64 ordered transport parts and a 16 MiB decoded
aggregate. Assembly identity, order, timeout, decoded length, and the complete
cross-referenced envelope are validated before admission. The aggregate holds
at most 262,144 spans and 4,096 actions. Text preparation also rejects a
15-word speech window with more than 512 combined client actions and internal
capitalization anchors before it reaches the synthesis queue. A decodable
invalid or stale direct timeline receives a terminal `failed` or `cancelled`
status; an undecodable record with no trustworthy dispatch identity is
diagnostic only.

The work queue admits without waiting for synthesis. A request is either
accepted with all required retirements or rejected without disturbing older
work. Under pressure, queued replaceable work may be evicted; ordered and
urgent work is never chosen for eviction. Tracked work receives a terminal
status for every retirement.

## Generations, replacement, and stop

Two cancellation mechanisms have different ownership:

1. **Hard interruption generation.** `s`, immediate `tts_say`, and letter
   speech advance the process generation. The worker rejects older requests
   before/after engine calls. Hard `s` stops all audio streams and all engines;
   immediate speech and letters stop the speech stream without clearing
   unrelated tone/sound output.
2. **Keyed replacement token.** A replaceable structured timeline prepares a
   token for `(protocol_version, replacement_key)`. Successful worker-queue
   admission atomically activates it and cancels the prior token in that
   domain. Failed admission does not disturb older queued or active work. The
   token does not advance the hard-stop generation and therefore cannot clear
   ordered, urgent, legacy, or another replacement domain.

Only replaceable structured timelines use the 20 ms quiet/80 ms maximum reader
window. Ordered and urgent timelines are submitted without reader debounce.
Adjacent timelines coalesce only when their policy-bearing version and
replacement key match; worker-queue replacement uses the same domain test.

The keyed token follows synthesis requests, rendered windows, deferred
overlays, playback cue delivery, and tracked completion. Queued or not-yet
started tagged sources disappear immediately on cancellation. Active speech
uses a three-millisecond frame-aligned fade to avoid a click. Unreached engine
markers, semantic events, and carried effect/overlay tails are discarded.
The cancellation lease remains alive until every tagged playback ticket is
terminal, preventing a late completion from removing a newer domain token.

## Engine registry and routing

Server mode eagerly registers available built-in engines. Windows retains
WinRT and eSpeak NG, then independently discovers optional adjacent Eloquence
and DECtalk helpers. macOS retains AVSpeechSynthesizer and eSpeak NG; Linux
retains eSpeak NG. A Piper-enabled build also registers Piper on every platform
when a model is configured. Startup selection chooses the initial preference
without removing the other registered engines.

The registry owns stable engine descriptors and physical voices. A separate
logical registry owns portable definitions; routing policy owns preferred,
fallback, and disabled engine lists. Registrations and policies are
generation-safe atomic replacements. A dispatched request snapshots them, but
the worker overlays current health immediately before synthesis.

For each speech chunk the router tries the logical selector/fallback sequence
and then the global policy. Missing voices, text outside an engine's lossless
repertoire, and bounded runtime failures can re-resolve the identical chunk.
Retries are capped. Persistent failure opens an engine circuit; cooldown and a
single recovery probe keep repeated requests on healthy fallbacks.

Immediate speech and letter commands have no logical-voice ID. They still use
the current global engine policy and runtime health snapshot.

## Native-call isolation and helper engines

WinRT and helper-backed engines are wrapped by a generation-aware isolated-call
boundary. It permits one active or quarantined call per engine and two across
the process. A cancelled native task cannot return PCM to the pipeline. If its
engine slot remains occupied after a bounded wait, routing chooses another
engine rather than queueing behind stale work.

Eloquence, DECtalk, and Piper use the versioned helper protocol. The main
server validates helper inventory, request/response order, PCM totals, markers,
and exact requested voice realization. A helper keeps reading cancellation and
health commands while its native synthesis worker runs. Piper uses libpiper's
chunked C API and observes stop requests between returned chunks. If any helper
cannot finish cancellation within the grace period, the host can terminate and
later recreate the child. The Piper helper disables Omnivox's separate eSpeak
backend so one process does not contain two interposing eSpeak runtimes.
Proprietary DLLs remain outside the repository.

See [HELPER-PROTOCOL.md](protocols/HELPER-PROTOCOL.md) and
[ENGINE-ISOLATION.md](ENGINE-ISOLATION.md).

## Text preparation and source offsets

Before synthesis Omnivox:

1. consumes the established `[*]` speech separator as a boundary space;
2. expands punctuation according to the active none/some/all level;
3. optionally inserts spaces at lower-to-uppercase CamelCase boundaries; and
4. chunks prepared text at a sentence, line, or clause boundary when possible,
   with a hard limit of 15 whitespace-delimited words.

Punctuation expansion is a route-independent compatibility contract. `none`
names only `$` and `%`. `some` names this complete set:

```text
! " # $ % ( ) * + - / : ; < = > \ ^ ` ~
```

`all` names every ASCII punctuation character. Characters not named at the
selected level, including non-ASCII punctuation, remain in the prepared text
so the synthesizer can retain natural phrasing. Both ordinary speech and
structured presentation timelines use this same preprocessing and produce the
same prepared text.

The `[*]` marker is compatibility text emitted by Emacspeak/Emacsvox character
names (for example `question[*]mark`); it must never reach punctuation
expansion as literal bracket/star characters.

Structured actions retain source UTF-8 offsets through preprocessing and
chunking. Offsets are mapped into the prepared chunk before the selected engine
resolves requested anchors. See [TEXT-CHUNKING.md](TEXT-CHUNKING.md).

## Audio and presentation ownership

All PCM is converted to stereo floating point at 44.1 kHz before the common
pipeline. Speech trimming reports removed frames so markers and anchors stay
aligned. Volume and channel routing are duration-preserving.

The pure timeline scheduler projects source/span positions to output frames.
Insertions advance the primary clock and shift later events; overlays do not
advance it but their tails extend tracked completion. The bounded renderer
processes one synthesis window at a time, caps its primary output at two
minutes, and carries overlay/effect tails into following windows.

File-action paths and parameters are validated before admission. After queue
admission, the worker decodes every file resource before synthesizing the first
span of that presentation, so a resource failure cannot play a new partial
prefix. Each file is limited to 16 MiB and 30 seconds. Immutable decoded PCM is
shared from an LRU cache capped at 128 entries and 64 MiB of `f32` samples; one
prepared presentation has its own 64 MiB retained-PCM budget, counting shared
allocations once and predicted private transformed copies. Generated tones and
silence remain bounded recipes and are materialized only for their render
window. Post-synthesis gain, low/high-pass filtering, pan, reverb, and echo
state persists across chunks and engine changes until explicitly replaced or
ended.

Speech, tone, and sound sinks can play concurrently. Within each sink sources
are ordered and bounded. Deferred legacy icons wait for their preceding speech
barriers but do not delay following speech; their tail still belongs to tracked
completion.

## Lifecycle invariants

- Parsing or validation failure cannot play a valid prefix of an atomic frame.
- An admitted newer keyed presentation affects only its exact replacement
  domain; failed admission leaves the older domain owner intact.
- Ordered and urgent work is never coalesced or evicted as replaceable work.
- Stale or cancelled PCM never enters playback.
- Unreached markers and semantic events never fire after cancellation.
- A tracked dispatch emits exactly one terminal result after all owned tickets
  are terminal and reached events are flushed.
- Display names are never stable engine/voice identity.
- Fallback may reduce optional capabilities, but it must not silently drop
  source text.
- Resource, queue, marker, frame, and helper payload limits remain explicit.

## Failure handling

Malformed input, engine failure, resource failure, and audio-queue failure are
reported without intentionally crashing the server. Tracked requests receive
`failed` or `cancelled`; untracked legacy failures go to diagnostics. A panic in
the sole synthesis worker is exceptional: the process logs a forced backtrace
and exits with status 70 so Emacs can replace the whole server rather than keep
a live control channel attached to a dead worker.

See [DIAGNOSTICS.md](DIAGNOSTICS.md) for evidence collection.
