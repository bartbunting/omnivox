# Omnivox Architecture

## Runtime boundary

Omnivox is a host-rendered speech server. Engines return PCM to the host, where
Omnivox canonicalizes, processes, schedules, and plays it. Buffered engines
return one complete result; protocol-v5 engines may instead supply bounded
canonical windows while native synthesis remains active. A future
external-playback backend would have reduced timeline and effects guarantees
and must advertise that difference explicitly.

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
omnivox-helper-host/   shared bounded lifecycle for native TTS helpers
omnivox-rhvoice-helper/ dynamically loaded user-installed RHVoice adapter
omnivox-flite-helper/  isolated Flite engine adapter and voice discovery
omnivox-flite-sys/     pinned portable C build and narrow native boundary
omnivox-rutts-helper/  isolated RuTTS adapter and UTF-8/KOI8-R boundary
omnivox-rutts-sys/     pinned portable C build and narrow native boundary
omnivox-tgspeechbox-helper/ isolated TGSpeechBox/eSpeak adapter
omnivox-tgspeechbox-sys/ narrow build-time TGSpeechBox C++ boundary
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
  - synthesize with cancellation checks and no-splice runtime fallback
  - canonicalize complete PCM or relay bounded progressive windows
  - render trimming, effects and timeline actions in bounded windows
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
and tones use a three-millisecond frame-aligned fade to avoid a click. Unreached
engine markers, semantic events, and carried effect/overlay tails are discarded.
The cancellation lease remains alive until every tagged playback ticket is
terminal, preventing a late completion from removing a newer domain token.

## Engine registry and routing

Server mode eagerly registers available built-in engines. Windows retains
WinRT and eSpeak NG, then independently discovers optional adjacent Eloquence
and DECtalk helpers. macOS retains AVSpeechSynthesizer and eSpeak NG; Linux
retains eSpeak NG. RHVoice, Flite, RuTTS, and TGSpeechBox companion helpers are
discovered on every desktop build when staged or explicitly configured. A
Piper-enabled build also registers Piper on every platform when a model is
configured.
Configured helpers initialize concurrently with built-in discovery, but the
server joins them and registers their descriptors in deterministic order before
opening its command loop. TGSpeechBox is the exception: its companion includes
source-identified descriptors for both supported native sample rates, generated
by the exact packaged helper. The server selects the configured rate's cache;
after bounded schema and descriptor validation, it registers that inventory
without blocking on the process. The live descriptor must exactly match the
selected cache. Once validation completes, a background pre-warm opens and
retains that helper connection while other initialization continues. A first
synthesis that overlaps pre-warming joins the same serialized lifecycle;
it never starts a duplicate process. A missing or invalid cache restores eager
initialization. The first inventory therefore remains complete while independent
helper process-start costs no longer accumulate serially. Startup selection
chooses the initial preference without removing the other registered engines.

When packaging supplies eSpeak data in a SHA-256-named directory with the
matching `omnivox-espeak-data.sha256` identity file, Omnivox may reuse a
bounded, schema- and eSpeak-version-checked voice inventory stored beside that
data. Only normalized voice records are cached; engine capabilities, health,
and the runtime default are reconstructed from the live engine. A missing,
oversized, malformed, mismatched, or non-content-addressed cache falls back to
native voice discovery and cannot make eSpeak unavailable.

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

Normalized speech rate is translated by a monotonic engine-specific curve
before native synthesis. The measured curves target the established Eloquence
English rate; RuTTS uses same-language Russian evidence, and an engine
saturates when its native rate control has no further headroom. Calibration is
based on canonical WAV duration, never engine startup or wall-clock synthesis
time. The policy and reproducible evidence procedure are in
[ADR 0004](adr/0004-per-engine-speech-rate-calibration.md) and
[RATE-CALIBRATION.md](RATE-CALIBRATION.md).

## Native-call isolation and helper engines

WinRT and helper-backed engines are wrapped by a generation-aware isolated-call
boundary. It permits one active or quarantined call per engine and two across
the process. A cancelled native task cannot return PCM to the pipeline. If its
engine slot remains occupied after a bounded wait, routing chooses another
engine rather than queueing behind stale work.

Eloquence, DECtalk, Piper, RHVoice, Flite, RuTTS, and TGSpeechBox use the
versioned helper protocol.
The main server validates helper inventory, request/response order, PCM totals,
markers, and exact requested voice realization. Protocol v5 can relay
interleaved marker and PCM frames through fixed-capacity isolation and playback
channels. Native mono/stereo helper PCM passes through one stateful sinc
converter, which retains less than one fixed input window between wire chunks
and maps native marker frames into the canonical playback clock. Older protocol
peers and engines needing whole-result operations stay on the buffered path.
The marker reporter reserves each progressive event before its cue is added to
playback, and silence-trimmed offsets are published before the corresponding
PCM window. Runtime retry is permitted before the first progressive PCM window
but never after it, preventing repeated or cross-engine speech splices. A helper
keeps reading cancellation and health commands while its native synthesis
worker runs. Piper uses libpiper's chunked C API and observes stop requests
between returned chunks. If any helper cannot finish cancellation within the
grace period, the host can terminate and later recreate the child. The Piper
helper disables Omnivox's separate eSpeak backend so one process does not
contain two interposing eSpeak runtimes. Proprietary DLLs remain outside the
repository.

RHVoice dynamically loads a user-installed 1.x C API runtime and keeps its
language/voice data outside Omnivox. Flite is source-built and statically linked
only into its SLT-only companion. RuTTS is likewise source-built only into its
companion; the adapter converts supported Unicode input to KOI8-R, expands its
signed 8-bit 10 kHz PCM into canonical samples, and ships without RuLex.
Experimental TGSpeechBox keeps its pinned C++ frontend/DSP and eSpeak IPA
conversion together in a GPLv3 helper, exposes only portable ACSS controls,
and advertises no markers. These helpers reuse the engine-neutral helper host
but never share a native process.

The shared 32-bit Windows C# host forwards native Eloquence and DECtalk callback
PCM for protocol v5 without retaining the complete waveform. The Rust receiver
uses one continuous high-quality conversion rather than changing the signal at
native callback boundaries. Eloquence can publish each index before its
following audio callback. DECtalk can report an index a few samples after the
callback containing that position, so its adapter holds exactly one 512-sample
native block and emits the next callback's markers before releasing it. Exact
Eloquence anchors and DECtalk anchors aliased to its existing word indexes feed
the incremental timeline renderer. That renderer carries overlays between
bounded windows, accounts for inserted audio when remapping later markers, and
publishes semantic events and actual anchor quality on the playback clock.
Routes whose streaming engine cannot resolve requested anchors retain the
whole-result path.

The Windows helpers require absolute native-library paths, validate x86 PE
identity and required exports before engine calls, and load dependencies only
from the selected library directory or System32. A missing or rejected runtime
is reported through the live helper protocol and does not become a helper
process crash.

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
window. Post-synthesis gain, low/high-pass filtering, pan, chorus, reverb, and
echo state persists across chunks and engine changes until explicitly replaced
or ended.

Speech, tone, and sound sinks can play concurrently. Within each sink sources
are ordered and bounded. Progressive speech remains one tracked source while a
fixed-capacity producer supplies PCM windows and frame cues. Before attaching a
progressive source to a real device, the producer primes three non-empty PCM
windows, or all available windows when a shorter source reaches its terminal.
Cue-only updates are retained by the producer and travel with the next PCM
window or terminal message, so they cannot displace this bounded audio reserve.
Natural completion requires an explicit producer terminal, while cancellation
closes the channel and preserves the speech de-click fade. A stream stop also
fades an active tone to zero while discarding queued tones without starting
them. Deferred legacy icons wait for their preceding speech barriers but do not
delay following speech; their tail still belongs to tracked completion.

The default output backend connects those sinks to the operating-system audio
device. An explicit null backend instead drains the same rodio source wrappers
as quickly as possible without opening a device. It therefore preserves queue,
cue, cancellation, overlay-barrier, and tracked-completion behavior while
attaching progressive sources immediately and deliberately removing real-time
device and acoustic timing from the run.

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
