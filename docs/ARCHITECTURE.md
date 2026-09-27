# Omnivox Architecture

This is the current implementation overview, reconciled on 2026-09-27.
The [decision index](adr/README.md) explains architectural choices;
[protocol references](README.md#protocol-specifications) define exact contracts;
[status](STATUS.md) records platform/runtime qualification and
[the roadmap](ROADMAP.md) tracks future work. Accepted design,
implemented behavior, native qualification and published releases are distinct.

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

## Process ownership and workspace

A local Emacsvox session uses separate foreground and notification workers.
Each owns its registry, queues, helper instances, cancellation and output lanes.
Helpers isolate native synthesis; the main worker handles audio conversion, effects,
scheduling and playback. Native libraries never move into the Emacs client.

The local `--voice-library-owner` path gates a worker behind established native
job/process-group ownership, captures exact startup configuration and acknowledges
retirement only after its tree and output reader exit. The local management
service coordinates installation and Apply; it is separate from speech admission.
See [local activation](guides/voice-management.md#local-provider).

An optional `--serve` broker authenticates loopback network connections before
spawning stdio workers. Each foreground/notification lane owns a worker and
its engine helpers; synthesis and PCM stay on the workstation. The broker
uses bounded records and handoffs, heartbeat leases, and tree retirement.
Unix workers have private process groups. Windows workers belong to private
jobs with kill-on-close semantics and cannot initialize engines before job
assignment completes. Remote workers use a restricted icon loader and cancel
on EOF; ordinary stdio workers retain their drain-on-EOF behavior. See
[ADR 0004](adr/0004-workstation-service-and-worker-ownership.md) and the
[remote protocol](protocols/remote.md).

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
linux-helpers/         dynamically loaded Linux ECI/DECtalk capture processes
third-party/           separately licensed, provenance-recorded native source
elisp/                 standalone upstream-Emacspeak compatibility adapter
```

The main server is `omnivox-cli`. Omnivox-specific data contracts live in
`omnivox-tts`; the audio crate defines the single canonical `AudioBuffer`. The
core crate remains independent of any one engine. The Windows and Linux legacy
engine helpers are separate executables with GPL-2.0-or-later source licenses.

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

The selected output runtime consumes audio asynchronously. Marker events
describe source frame consumption, not guaranteed first output from the
physical device. The tracked
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
uses a three-millisecond frame-aligned fade, while active tones use a
five-millisecond smoothstep fade to further suppress cut-off clicks. Unreached
engine markers, semantic events, and carried effect/overlay tails are discarded.
The cancellation lease remains alive until every tagged playback ticket is
terminal, preventing a late completion from removing a newer domain token.

## Configuration and engine discovery

Launch configuration currently comes from CLI options, documented environment
variables, packaged helper locations and optional immutable voice-library
inputs. Engine IDs, helper path mappings and startup order are still enumerated
in [`omnivox-cli/src/engine.rs`](../omnivox-cli/src/engine.rs). There is no generic
external-helper manifest reader or `config.json` policy reader yet. The
[configuration reference](guides/configuration.md) describes existing option semantics.

Server startup attempts eSpeak NG on all desktops plus WinRT on Windows or
AVSpeechSynthesizer on macOS. Known companions are discovered from staged paths
or explicit overrides. Piper additionally requires compiled support and a
legacy model or managed selection. MBROLA requires its explicit absolute helper
path; adjacent placement alone does not enable it. `--engine` or the existing
environment selection sets an ordinary startup preference, while other eligible
engines remain registered for fallback. Exact diagnostics target one engine.

Helpers initialize concurrently with built-in discovery and join the registry
in deterministic order before the command loop. TGSpeechBox can register a
bounded, source-identified packaged descriptor cache for its native sample rate
and prewarm one connection in the background. First synthesis joins that same
serialized connection; its live descriptor must match the cache. Invalid/missing
cache restores eager discovery. Verified content-addressed eSpeak data can reuse
bounded version-checked voice records; capabilities, health and default identity
are reconstructed from the live engine. Invalid cache never disables live discovery.

Failed configured helpers retain an unavailable entry with a reason and no
invented voices. Asynchronous recovery can rescan them under one bounded attempt
per helper; managed rescans repeat asset and exact-inventory checks. Registry
reads use an atomic cached descriptor/generation snapshot and perform no engine
initialization. Successful discovery publishes descriptor and handle together.

## Voice identity, routing and tuning

A physical voice is `(engine_id, voice_id)`. A logical voice holds ordered
selectors or stable choice records with settings. Display names are not identity.
Logical registrations and routing policy use generation-safe atomic replacement.
Admission captures registry, policy, exclusions, host rate and span context;
health is refreshed before actual synthesis without replacing those inputs.

The resolver tries ordered logical selectors, optional same-language fallback
on the first requested engine, preferred-engine defaults, the global default
selector and fallback-engine defaults. Text-repertoire checks reject known
encoding loss. Runtime retries prepare the identical chunk, are capped at four,
and stop at output/PCM commitment. Health circuits, cooldowns and a single probe
keep repeated failures away from normal speech. Immediate/letter commands use
global policy without a named logical voice.

Language properties match case-insensitive exact tags: `en` does not mean
`en-AU`. The logical language only constrains the corresponding language fallback;
preferred/fallback defaults do not automatically preserve it. Inventory grouping
and an engine's language-switching flag are not language detection or general
per-span language routing. Changed matching semantics remain future work.

Layered requests compose shared settings, the actual selected choice's patch,
then span context. Policy fallback has no choice patch. Each attempt starts from
its selected adapter's qualified defaults, applies common controls once, and
prepares its effects with the route identity. Failed attempts cannot leak settings
or effects. Legacy/layered boundaries separate effect ownership. Measured common
rate curves and native saturation follow [rate calibration](reference/rate-calibration.md).

Qualified native controls use typed adapter catalogues and sparse edits. Catalogue
queries are bounded/read-only and connection/runtime scoped; they do not load all
models or restart speech. Runtime replacement invalidates metadata qualification.
The actual helper validates native application; marker-4 receipts describe the
choice/application at first consumed PCM, not merely a predicted route or successful
parameter query. See the [native contract](protocols/engine-voice-parameters.md).

Exact previews preserve the exact physical target. Complete previews use a private
voice/policy/context snapshot and may perform its permitted fallback without
mutating applied configuration. Terminal evidence distinguishes attempts,
accepted PCM and source starts. Both speech workers negotiate capabilities and
acknowledge independently. The
[prepared-attempt reference](reference/prepared-synthesis.md) describes execution
and ticket ownership; the [control protocol](protocols/control.md) specifies
versioned operations, bounds and compatibility.

Bundled eSpeak variants are derived on demand from a bounded live suffix
catalogue. Explicit combinations validate exact native identity and exclusions;
automatic/property matching keeps the compact base inventory. Selecting a variant
neither mutates a managed load set nor restarts workers. See
[eSpeak variants](engines/espeak-variants.md).

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
between returned chunks; because libpiper exposes no synchronization markers,
marker-dependent presentation still collects its result. If any helper cannot
finish cancellation within the grace period, the host can terminate and later
recreate the child. The Piper
helper disables Omnivox's separate eSpeak backend so one process does not
contain two interposing eSpeak runtimes. Proprietary DLLs remain outside the
repository.

Adapters retain platform-specific native ownership behind this common contract:

| Adapter | Native responsibility / current boundary |
| --- | --- |
| RHVoice | User-installed C API library, source-mapped SSML marks and callback PCM. Runtime and external/managed data remain separate. |
| Flite | Serialized process-global runtime, selected SLT/external voices, progressive callbacks after native word-marker metadata is available. |
| RuTTS | Lossless KOI8-R conversion and continuous conversion of signed 8-bit 10 kHz callbacks. No RuLex or markers; anchor-dependent work stays buffered. |
| TGSpeechBox | Pinned eSpeak IPA frontend and native DSP share its dedicated process. Caller indexes give exact requested anchors, not general linguistic source ranges. |
| eSpeak NG | In-process callback PCM; native SSML marks with generated-text source maps for anchored requests, ordinary text path otherwise. |
| macOS | Cocoa owns capture and native serialization; Rust drains a bounded callback queue through the common converter. Cancellation wakes backpressure waits; explicit native completion ends production. Unconfirmed retirement quarantines the owner. No native markers are advertised. |
| Windows Eloquence/DECtalk | Separate x86 C# executables with restricted library loading and architecture/export checks. Qualified helper-6 controls extend the retained older protocol paths. Eloquence publishes indexes ahead of audio; DECtalk holds one native block to publish late indexes before its PCM. |
| Linux ECI/DECtalk | Dedicated native owner threads, absolute architecture-checked ELF libraries and bounded cancellation-aware PCM/marker queues. ECI aborts via its callback; DECtalk coordinates reset outside callback locks. System dependencies remain linker-owned. |
| MBROLA | Explicit private helper owns sequential frontend/runtime children, per-request verified inputs and bounded buffered output. It advertises no streaming or markers. |

The [engine guides](README.md#engine-guides) own ABI details, runtime
loading and platform acceptance. Component supply and licensing remain independent
of helper protocol capability under [ADR 0001](adr/0001-engine-isolation-and-distribution.md).

See [helper.md](protocols/helper.md) and
[native-call-isolation.md](reference/native-call-isolation.md).

## Installed assets and activation

The voice library separates installed files, desired enablement, immutable runtime
generations and active workers. Omnivox's local service manages bounded acquisition,
storage, validation, removal and native worker ownership. Emacsvox supplies the
reviewed catalogue and coordinates explicit Apply for its two speech lanes.
Downloads/imports start disabled; install/enable alone does not restart speech.

Piper holds at most one model per helper and shares it across speakers. Flite
loads its selected voices. MBROLA validates the selected database and common
frontend before its short-lived native child. RHVoice separates the user's runtime
from managed voice/language packages. Unsupported provider schemas fail before
activation; explicit legacy overrides remain visible and cannot erase exclusions.

Native validation uses private load projections, exact staged helpers, real PCM,
asset checks and bounded supervised cleanup without playback. Saved observations
bind inputs and cleanup; they do not permit skipping validation or attest every
loaded system library. Persistent leases, operation journals and live supervisors
retain ownership across manager loss. Incomplete cleanup blocks conflicting work;
a saved PID alone grants no authority to kill a later process.

Apply preflights a candidate, retains both exact startup/rollback configurations,
establishes owned worker trees and verifies both workers before active-pointer
publication. Partial failure retains cleanup ownership and uses the saved rollback
configuration. Ordinary startup may retain an optional provider as unavailable
and use eligible fallback, while exact validation and candidate Apply remain
strict. Malformed generation metadata fails rather than dropping exclusions.

Removal checks ownership and active/rollback/session references, preserving
external files, runtimes and saved palette choices. Stronger crash/power-loss
recovery and platform qualification remain explicit limits, not implied by a
successful generation acknowledgement. The
[voice-library contract](reference/voice-library.org),
[installation guide](guides/voice-management.md),
[validation guide](guides/native-voice-validation.md) and
[removal guide](guides/voice-management.md#managed-voice-uninstallation) own their detailed formats and operations.

## Text preparation and source offsets

Before synthesis Omnivox:

1. consumes the established `[*]` speech separator as a boundary space;
2. expands punctuation according to the active none/some/all level;
3. optionally inserts spaces at lower-to-uppercase CamelCase boundaries; and
4. chunks prepared text at a sentence, line, or clause boundary when possible,
   with a hard limit of 15 whitespace-delimited words.

Punctuation expansion is route-independent and shared by legacy and structured
speech. The compatibility separator never reaches punctuation expansion as
literal markup. Structured actions retain original UTF-8 offsets through text
preparation and chunking; the selected engine resolves mapped anchors. Exact
punctuation sets, segmentation and offset rules live in
[text-chunking.md](reference/text-chunking.md).

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
Explicit legacy letter navigation can also release the reserve once 40 ms of
rendered PCM is ready, retaining the three-window and short-terminal conditions.
This frame threshold is not a timer and does not apply to ordinary speech or
previews. [ADR 0003](adr/0003-progressive-audio-and-markers.md) records the decision
and the [matched letter report](benchmarks/2026-09-20-letter-playback-reserve.md)
records device-source timing and observed stalls.
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

Linux also has an opt-in native `pulse` backend governed by
[ADR 0005](adr/0005-native-pulseaudio-output.md). The same source wrappers feed
three independent PulseAudio streams, which the server mixes on its default
sink. Each has a source worker and native event thread. It requests 20 ms
buffering, writes about 5 ms at a time, drains/corks when idle, and retires a
failed connection's queued sources. Fresh audio may reopen a failed lane after
a short admission cooldown; reconnect runs on its source worker, with no idle
retry loop or replay of failed speech. Consumer errors do not quarantine a
healthy synthesis engine. A stream-wide stop discards local PCM and
flushes that PulseAudio lane immediately; selective request cancellation keeps
the shared source fade without flushing unrelated work. Marker/completion
semantics remain source-based. WSLg still forwards PulseAudio output over RDP.

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

See [diagnostics.md](guides/diagnostics.md) for evidence collection.

## Proposed extensions

The [extensible-engine framework](plans/extensible-engine-framework.md) proposes
shared launch registration, `helpers.d/` manifests and `config.json` policy.
Its [ADR 0008](adr/0008-extensible-engine-registration.md) is accepted for v1; these
readers, coordinated helper-launch snapshots and external automatic-selection
permissions are not current features. Richer language matching is a separate
future increment. The [roadmap](ROADMAP.md) tracks outstanding work.

Performance claims belong to the [retained evidence](benchmarks/README.md).
Source consumption, protocol success and process liveness do not prove acoustic
output; platform/runtime qualification and listening remain separately recorded.
