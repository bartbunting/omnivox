# Omnivox Control Protocol

Omnivox retains the Emacspeak line protocol for speech presentation and adds a
separate versioned channel for discovery, configuration, and diagnostics. The
control channel uses Base64-encoded UTF-8 JSON so native engine and voice IDs do
not need Tcl escaping and remain separate structured fields.

## Compatibility Boundary

- Existing clients and legacy commands continue to work unchanged.
- `emacsvox_tx` is an optional presentation-transaction frame. It is not used
  for persistent logical-voice configuration or discovery.
- `presentation_tone_v1` gates `emacsvox_tone 1 MODE FREQUENCY DURATION`;
  `insert` advances the speech clock and `overlay` starts at its current
  boundary. The ordinary `t` command remains an independent beep.
- A client must successfully request capabilities before using later control
  extensions.
- The server advertises only features it currently implements. Control
  protocol version 1 covers capability negotiation, active-engine inventory,
  atomic logical-voice registration, queued logical-voice routing, exact or
  portable one-shot voice preview, generation-safe runtime engine policy,
  explicit recovery probes, legacy framed transactions, tracked playback, and
  presentation capability discovery. Structured timelines and marker events
  retain their own advertised protocol versions.

## Request Record

A client writes one ordinary protocol command:

```text
omnivox_control {BASE64_JSON}
```

The decoded JSON envelope contains a protocol version, a client-chosen request
ID, and a tagged request body. The current request is:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "capabilities"
}
```

The Base64 field must not contain line breaks. Omnivox rejects an encoded field
before decoding if it cannot fit within the 256 KiB decoded-payload limit.

Every main-protocol record, including `omnivox_control`, is limited to 512 KiB
of UTF-8 before the line ending. This is above the maximum valid single-frame
Base64 control and presentation records. Omnivox drains and rejects an
oversized or non-UTF-8 line, then resumes with the next record. Its reader
handoff buffers at most 32 complete lines.

Pending unframed legacy transactions are limited to 4,096 queue items and 16
MiB of text/resource payload. A transaction that exceeds either limit is
discarded atomically at dispatch. The synthesis handoff admits at most 32
waiting requests and 32 MiB of estimated owned payload without blocking the
protocol loop. Matching replaceable timelines can coalesce, and other queued
replaceable timelines can be cancelled under capacity pressure. Ordered and
urgent work is not evicted; if it occupies the available capacity, incoming
work fails. Tracked requests receive `cancelled` or `failed`, previews receive
their corresponding terminal control response, and untracked legacy rejection
is reported on standard error. Stop/reset cancels queued older generations
before accepting subsequent work.

The structured inventory request uses the same envelope with `"type":
"inventory"`.

A client replaces its complete logical-voice registry with a registration
request. Clients should begin at generation 1 and increment the generation
whenever the definitions or fallback policy change:

```json
{
  "protocol_version": 1,
  "request_id": 43,
  "type": "register_logical_voices",
  "registry_generation": 1,
  "definitions": [
    {
      "id": "source-code",
      "language": "en-US",
      "preferences": [
        {
          "kind": "exact",
          "engine_id": "winrt",
          "voice_id": "winrt:David"
        },
        {
          "kind": "engine_default",
          "engine_id": "espeak"
        }
      ],
      "acss": { "rate": 0.6, "average_pitch": 0.4 }
    }
  ],
  "fallback_policy": {
    "allow_same_language_on_requested_engine": true,
    "global_default": null,
    "fallback_engines": ["espeak"]
  }
}
```

Omitting `fallback_policy` selects the empty default policy.

When `runtime_routing_policy` is advertised, engine policy is a separate atomic
generation from logical-voice registration:

```json
{
  "protocol_version": 1,
  "request_id": 44,
  "type": "set_routing_policy",
  "routing_policy_generation": 1,
  "preferred_engine_ids": ["eloquence", "dectalk", "winrt"],
  "fallback_engine_ids": ["espeak"],
  "disabled_engine_ids": ["dectalk"]
}
```

The preferred order is considered after a logical voice's explicit selectors
and requested-engine same-language fallback, but before the global default and
fallback-engine list. Unqualified property selectors also use it to rank
otherwise matching engines. Disabled engines
remain in their configured positions and reappear there when restored, but are
projected as unavailable while disabled. Unknown but syntactically valid engine
IDs are retained so the same machine profile can degrade and later recover as
engines appear. An identical generation retry is idempotent; an older
generation or different content at the current generation is rejected without
changing routing.

When `exact_voice_preview` is advertised, a client may audition one selector
and unsaved normalized ACSS style without changing the registered logical
voices or persistent speech state:

```json
{
  "protocol_version": 1,
  "request_id": 45,
  "type": "preview",
  "text": "Compare this voice using identical text.",
  "selector": {
    "kind": "exact",
    "engine_id": "eloquence",
    "voice_id": "eci:Reed"
  },
  "language": "en-AU",
  "acss": { "average_pitch": 0.4 },
  "rate_offset": -4,
  "effects": { "reverb": 0.2 }
}
```

The preview text is limited to 16 KiB. An exact selector is strict: it does not
inherit the active logical registry or its fallback policy. A property or
engine-default selector remains portable and may resolve again within that
same selector if its first runtime target fails.

When `relative_rate_v1` is advertised, previews and presentation speech spans
may include a signed integer `rate_offset` from `-20` through `20`. Omnivox adds
`rate_offset / 100` to the stored normalized host rate, then clamps the derived
one-shot rate to the normalized ACSS `0.0..1.0` range without changing the
stored rate. Expressed on the host's integer point scale, base 75 plus `-1` is
74 and base 75 plus `4` is 79. An extended host rate above 100 therefore remains
at 100 unless a negative offset brings it below that ceiling. A request must not
combine `rate_offset` with the absolute `acss.rate` field. Zero is neutral and
should normally be omitted.

## Response Record

The server writes one flushed, newline-terminated event to standard output:

```text
__OMNIVOX_CONTROL__ BASE64_JSON
```

A successful capability response decodes to this shape:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "capabilities",
  "server_version": "SERVER_VERSION",
  "supported_protocol_versions": [1],
  "features": [
    "control_v1",
    "engine_inventory",
    "engine_recovery_probe",
    "emacsvox_tx",
    "exact_voice_preview",
    "legacy_commands",
    "logical_voice_registration",
    "logical_voice_language_routing",
    "logical_voice_routing",
    "playback_marker_events_v1",
    "playback_marker_events_v2",
    "presentation_timeline_v1",
    "presentation_timeline_v2",
    "presentation_timeline_v3",
    "presentation_tone_v1",
    "post_synthesis_effects_v1",
    "preferred_engine",
    "process_audio_routing",
    "relative_rate_v1",
    "runtime_routing_policy",
    "stable_voice_ids",
    "text_repertoire_routing_v1",
    "tracked_playback_completion"
  ],
  "deprecated_commands": [
    "set_lang",
    "set_next_lang",
    "set_previous_lang",
    "set_preferred_lang",
    "tts_set_notification_channel"
  ]
}
```

`SERVER_VERSION` above is a documentation placeholder for the running binary's
workspace version, not a literal protocol value.

A preview produces exactly one request-owned terminal response. Omnivox emits
it only after every preview playback ticket completes, is cancelled by `s`, or
fails:

```json
{
  "protocol_version": 1,
  "request_id": 45,
  "type": "preview_completed",
  "status": "completed",
  "requested": {
    "kind": "exact",
    "engine_id": "eloquence",
    "voice_id": "eci:Reed"
  },
  "realized": {
    "engine_id": "eloquence",
    "voice_id": "eci:Reed"
  },
  "degraded_acss": ["pitch_range"],
  "degraded_effects": [],
  "message": null
}
```

`status` is `completed`, `cancelled`, or `failed`. `realized` is `null` when
resolution never found a usable target. Unsupported ACSS and post-synthesis
effect dimensions are listed without preventing otherwise valid speech.
Preview uses the ordinary buffered synthesis and mixer path, but it owns a
private one-definition routing snapshot and does not replace logical
registration, change server defaults, or affect a notification process.

The request ID lets Emacsvox distinguish simultaneous main and notification
process responses. Server events are never injected into synthesized speech.

An inventory response contains an `inventory_generation`, the compatibility
`preferred_engine_id`, the complete routing-policy generation and lists, an
`engine_runtime` array, and an `engines` array. Each engine includes its stable
ID, runtime availability and health, capabilities, discovered physical voices,
and default voice ID. Dynamic status reports the circuit state (`closed`,
`cooldown`, `ready`, or `probing`), last runtime failure, remaining cooldown in
milliseconds, and policy disablement separately from static capability. Server
mode eagerly registers available built-in engines: WinRT and eSpeak on Windows,
AVSpeechSynthesizer and eSpeak on macOS, and eSpeak on Linux. A Piper-enabled
server also registers Piper when a model is configured; Windows additionally
discovers configured Eloquence and DECtalk helpers. `OMNIVOX_ENGINE` changes
the initial preference without removing another available engine from
inventory. Descriptors are snapshotted before the reader loop starts, so
inventory requests cannot block stop commands behind synchronous synthesis.
Registry inventory is sorted by stable engine ID, and its generation advances
when engines or their descriptor state change. Persistent runtime circuit
state is overlaid on those snapshots; inventory generation also advances when
an engine fails, becomes probe-ready, starts probing, or recovers.

Engine capabilities include `text_repertoire`: `unicode`, `windows_1252`,
`iso_8859_1`, or `unknown`. The last value is also the compatibility default for
an older descriptor that omitted the field and guarantees only ASCII to the
router. This capability describes lossless input encoding, not pronunciation
quality or language support.

The complete [validated inventory response fixture](../protocol-fixtures/control-inventory-response.json)
shows every nested engine, capability, runtime, voice, availability, health,
and routing-policy field. A repository test deserializes it through the public
Rust response type so field and enum spelling changes cannot silently leave the
example behind.

A successful policy update returns `routing_policy_applied`, the effective
policy generation and content, and every logical voice re-resolved against the
new policy. Dispatched batches retain their policy snapshot; later changes
affect only later dispatches. The same policy also chooses the engine for
legacy queued speech, immediate speech, and letter commands when no logical
route is active.

When `engine_recovery_probe` is advertised, a client can move a failed engine
out of cooldown so the next routed request performs its engine-specific
recovery preparation and synthesis probe:

```json
{
  "protocol_version": 1,
  "request_id": 46,
  "type": "request_engine_recovery_probe",
  "engine_id": "dectalk"
}
```

The request is rejected for an unknown, policy-disabled, healthy, or already
probing engine. Probe success closes the circuit; failure returns it to bounded
cooldown and uses the same chunk's configured fallback.

A successful registration response reports the inventory generation used and
one resolved or unresolved binding for every definition:

```json
{
  "protocol_version": 1,
  "request_id": 43,
  "type": "logical_voices_registered",
  "inventory_generation": 1,
  "registration": {
    "registry_generation": 1,
    "bindings": [
      {
        "status": "resolved",
        "resolution": {
          "logical_voice_id": "source-code",
          "requested": {
            "kind": "exact",
            "engine_id": "winrt",
            "voice_id": "winrt:David"
          },
          "realized": {
            "engine_id": "winrt",
            "voice_id": "winrt:David"
          },
          "reason": "preferred",
          "failed_attempts": []
        }
      }
    ]
  }
}
```

Registration is an atomic whole-set replacement. A newer generation validates,
normalizes ACSS values, replaces the stored definitions, and resolves them
against the current inventory. An identical retry of the current generation is
idempotent and re-resolves against current availability. Reusing that
generation with different content or sending an older generation is rejected
without changing the registry. A valid but presently unresolvable definition
is retained and returned with `"status": "unresolved"` and diagnostic attempts;
it is not silently dropped.

At synthesis time the resolved engine's current descriptor filters the logical
ACSS record. Supported rate, average pitch, and volume values map into the
common synthesis settings. Pitch range, stress, and richness travel in the
structured request and are mapped by engines that advertise those dimensions;
the Eloquence and DECtalk helpers provide native mappings. Unsupported
dimensions are omitted without preventing speech. A runtime fallback
recomputes this application for the replacement engine instead of reusing the
failed engine's capabilities.

## Legacy Speech-Text Separator

Emacspeak and Emacsvox character-name tables use `[*]` as an in-band speech
boundary, for example `question[*]mark`. It is speech text, not timeline or
control framing. Omnivox consumes the complete marker as a space before
punctuation expansion at every punctuation level. Its component `[`, `*`, and
`]` characters must therefore never be announced independently. Structured
source offsets that cross the marker are remapped to the collapsed boundary.

## Presentation Transactions

When `emacsvox_tx` is advertised, a client may send one replaceable
presentation as:

```text
emacsvox_tx 17 {BASE64_LEGACY_SCRIPT}
```

The generation is a positive, monotonically increasing integer. The decoded
payload is a UTF-8 script containing ordinary legacy protocol commands, ending
with exactly one `d` dispatch command. Omnivox decodes, parses, bounds, and
validates the complete script before executing any of it. A payload may contain
at most 256 KiB and 4096 commands. Stop, control, exit, nested transaction,
reset, version, letter, immediate speech, and immediate sound commands are not
valid inside a frame.

Stale or repeated generations are ignored. Consecutive valid frames arriving
within the server's short reader coalescing window are replaceable: only the
highest generation executes. An ordinary command closes that window and runs
after the selected frame. An immediate `s` stop instead discards the selected
frame, performs the stop, and consumes the selected generation so that it
cannot reappear on retry. Invalid frames do not consume their generation and
may be corrected and retried.

This extension is the atomic compatibility transport for legacy server-command
scripts. Current Emacsvox uses the version 3 structured timeline for modeled
Aural presentation and may use this frame only on an explicit legacy lowering
path. It keeps sending the ordinary legacy protocol to older servers. Because
`emacsvox_tx` carries no dispatch identifier, frame validation failures are
server diagnostics rather than tracked response records.

## Tracked Playback Completion

When `tracked_playback_completion` is advertised, a client may dispatch the
current legacy queue with a positive identifier:

```text
emacsvox_tracked_dispatch 73
```

This command replaces the ordinary `d` for that dispatch. It is deliberately
not valid inside an `emacsvox_tx` payload. For every accepted identifier the
server writes exactly one flushed terminal record:

```text
__EMACSVOX_TRACKED__ 73 completed
__EMACSVOX_TRACKED__ 73 cancelled
__EMACSVOX_TRACKED__ 73 failed
```

`completed` means the batch succeeded and every nonempty audio buffer it queued
across the speech, tone, and sound streams was consumed to its natural end. An
accepted empty current-generation dispatch also completes. This is a mixer
source-exhaustion guarantee; it is not proof that a physical output device was
audible.

`cancelled` means the generation became stale before or during processing, or
a queued source was cleared or dropped before its natural end. Stop commands,
backlog clearing, and audio-sink teardown therefore cancel affected tracked
playback. `failed` means synthesis, fallback resolution, audio-icon loading,
effects processing, or audio queuing failed. The reporter waits for any audio
that was successfully queued before emitting the result, and `failed` takes
precedence if that audio is later cancelled.

Clients that do not see the capability must continue using ordinary `d` and
must not infer playback completion from synthesis or command acceptance.

## Marker-Aware Playback

When `playback_marker_events_v1` is advertised, a client may replace `d` with
the following positive-ID dispatch command:

```text
emacsvox_marker_dispatch 91
```

This is a separate capability and command so existing users of
`emacsvox_tracked_dispatch` never receive unsolicited records. Marker-aware
dispatch retains the same terminal contract and writes exactly one final
`__EMACSVOX_TRACKED__ 91 STATUS` line.

Before that terminal line, Omnivox writes zero or more flushed marker records:

```text
__EMACSVOX_MARKER__ BASE64_JSON
```

Each decoded event contains `protocol_version`, `dispatch_id`, and a one-based
`sequence`. Version 1 emits an `utterance_started` event when playback consumes
the first frame of every nonempty synthesized chunk:

```json
{
  "protocol_version": 1,
  "dispatch_id": 91,
  "sequence": 1,
  "type": "utterance_started",
  "utterance_id": 1,
  "text": "hello world",
  "engine_id": "winrt",
  "actual_voice": {"engine_id": "winrt", "voice_id": "David"},
  "logical_voice_id": "source-code",
  "sample_rate": 44100,
  "frame_count": 22050
}
```

The text is the exact chunk sent to the realized engine after Omnivox
preprocessing and chunking. `logical_voice_id` and `actual_voice` are nullable
when no logical route was requested or the engine cannot report an exact
voice. The start event exists even when an engine supplies no native markers,
which makes route and utterance timing observable without overstating engine
capabilities.

An engine marker produces a `marker_reached` event referring to that
`utterance_id`:

```json
{
  "protocol_version": 1,
  "dispatch_id": 91,
  "sequence": 2,
  "type": "marker_reached",
  "utterance_id": 1,
  "marker": {
    "kind": "word",
    "frame_offset": 4410,
    "text_start": 0,
    "text_length": 5,
    "value": "hello"
  }
}
```

Marker kinds are `word`, `sentence`, `phoneme`, and `native_index`. Text ranges
are optional UTF-8 byte ranges in the associated start event's `text`.
`frame_offset` uses the advertised canonical sample rate and has already been
adjusted for resampling and silence trimming. Event sequence and same-frame
marker order are stable across the dispatch.

Events report mixer source consumption and may lead acoustic output by the
audio device's buffering latency. Stop, backlog clearing, or source teardown
drops unreached events. A writer barrier drains every reached event before the
terminal status record, including a marker at the final frame boundary. One
decoded event is bounded to 2 MiB. The server losslessly bounds outstanding
marker-event output to 8,192 records and 16 MiB of serialized records, applying
backpressure to event producers rather than dropping reached events. Invalid,
oversized, or unrecognized events must be ignored by a client without speaking
their raw protocol text.

`playback_marker_events_v2` uses the same command, prefix, dispatch ownership,
ordering, payload bound, and flush barrier. It retains version 1 utterance and
engine-marker records and adds timeline action resolution, semantic events,
and style-degradation records. Its additional record shapes are specified in
[PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md).

## Structured Presentation Timelines

Capabilities `presentation_timeline_v1` through
`presentation_timeline_v3` gate the structured Aural transport. Current
Emacsvox requires version 3 for delivery policy and multipart semantics. A
direct frame with a trustworthy generation/dispatch identity receives
`failed` when its decoded envelope is invalid and `cancelled` when stale; a
record that cannot be decoded far enough to establish ownership remains a
diagnostic-only rejection.

The envelope, multipart framing, policy, resource, source-position,
marker-v2, and degradation contracts are maintained in
[PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md).

## Queued Logical-Voice Routing

When `logical_voice_routing` is advertised, a client selects a registered
logical voice inside the existing queued code command:

```text
c {[[logical_voice source-code]] [[pitch 1.2]]}
q {let answer = 42;}
d
```

The directive changes the engine and physical voice used by following queued
speech spans. It composes with legacy pitch codes and remains active until a
later logical-voice directive or a legacy physical `[[voice ...]]` code changes
the route. A server that predates this feature ignores the unknown directive
and continues to apply the legacy codes.

Definitions, fallback policy, runtime engine policy, and administrative
disablement are snapshotted when a batch is dispatched. The worker resolves
them against its current health-adjusted inventory before speaking the batch,
so an engine failure or recovery that happened while the batch waited in the
worker queue is respected. A registration or policy update received later
still affects only later batches. An unknown, unresolved, or
no-longer-registered route degrades to the preferred legacy engine and voice
instead of dropping speech. Hard stop and reset requests cancel playback and
request cancellation from every registered engine.

Logical voice IDs still apply to queued `q`/`c` speech only. Immediate
`tts_say` and `l` commands use the runtime global engine order but do not select
a logical voice. For a queued logical route, a runtime `VoiceNotFound` excludes
that physical voice and an unavailable
or failed synthesis call excludes that engine from the dispatched batch's
inventory snapshot. Omnivox then re-runs the registered definition and fallback
policy and retries the identical text chunk on the newly resolved route.

Before each synthesis call, Omnivox projects engines that cannot encode that
chunk as unavailable and resolves the same logical definition against the
remaining inventory. This does not mark an engine unhealthy: a later compatible
chunk returns to the preferred route. The exact UTF-8 text and requested anchor
offsets are retained on the fallback route, and marker events expose the
realized engine and voice. If no configured route can preserve the text, the
chunk fails explicitly without calling an incapable engine.
`text_repertoire_routing_v1` advertises this behavior so a client can stop
compatibility name expansion only after every live speech stream confirms it.

Each chunk is limited to four total synthesis attempts, with the request
generation checked before and after every attempt. A stop therefore prevents a
failed call from reappearing through its fallback. Invalid parameters are not
route failures and are not retried. If fallback is exhausted, Omnivox logs the
failure, skips later speech under that failed logical route, continues other
queued item types, and keeps the server alive. Voice-not-found exclusions stay
batch-local because another voice on the same engine may still work.

Unavailable and failed synthesis calls also open a persistent engine circuit.
Cooldowns are 5, 15, 30, and at most 60 seconds across consecutive failures.
While open, new batches resolve around the engine and inventory reports it as
failed with a recovery-pending reason. After the cooldown, inventory reports it
as degraded and exactly one routed synthesis request receives a recovery-probe
permit; other requests continue to route around a probe in progress. Probe
success closes the circuit and restores the preferred route. Probe or recovery
preparation failure reopens it and retries the same chunk on a fallback.
Cancellation releases a reserved probe without increasing the failure count.

Before a recovery probe, Omnivox calls the engine's recovery-preparation hook.
In-process engines use the safe no-op default. The generic helper-backed engine
invalidates a failed child and uses this hook to start and negotiate a fresh
helper before synthesis; Eloquence and DECtalk both use that implementation.

## Errors

Errors use the same response envelope:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "error",
  "code": "unsupported_version",
  "message": "unsupported control protocol version 2; supported version is 1"
}
```

The error codes are:

- `malformed_request`: invalid Base64, UTF-8, JSON, or request shape;
- `unsupported_version`: a well-formed request uses an unsupported version;
- `unsupported_operation`: a recognized deprecated legacy command has no
  supported state mutation and includes migration guidance;
- `payload_too_large`: the encoded or decoded size bound was exceeded;
- `invalid_configuration`: a registration violates ID, count, or field bounds;
- `stale_generation`: the registration generation is older than the stored
  generation;
- `generation_conflict`: the stored generation was reused with different
  content.

If decoding fails before the request ID can be trusted, `request_id` is `null`.
Errors do not mutate speech, queue, engine, or logical-voice state. A
registration accepts at most 256 definitions, 32 selectors per definition, and
a 256 KiB decoded JSON payload. Logical and engine IDs are bounded ASCII tokens;
physical voice IDs may contain native punctuation but not control characters.

## Structured Voice Data

The Rust engine, voice, logical-definition, selector, fallback, ACSS, and
resolution types have stable JSON representations for subsequent version 1
messages. The three selector variants are:

```json
{"kind":"exact","engine_id":"winrt","voice_id":"winrt:David"}
{"kind":"engine_default","engine_id":"espeak"}
{"kind":"properties","engine_id":null,"language":"en-AU","gender":"female"}
```

`gender` is `female`, `male`, `neutral`, or `null`. A property selector may
also omit its engine, language, or gender by sending `null`. Exact physical
identities keep engine and native IDs as separate fields:

```json
{
  "kind": "exact",
  "engine_id": "winrt",
  "voice_id": "winrt:HKEY_LOCAL_MACHINE\\SOFTWARE\\...\\David"
}
```

No consumer should construct or parse a combined `engine:voice` string.
Display names are presentation only and must not be used as stable identity.

Availability and health are tagged objects rather than strings:

```json
{"status":"available"}
{"status":"unavailable","reason":"disabled by runtime routing policy"}
{"status":"healthy"}
{"status":"degraded","reason":"recovery pending"}
{"status":"failed","reason":"native engine failed"}
```

Concurrency is `{"mode":"serialized"}` or
`{"mode":"concurrent","maximum_requests":4}`; `maximum_requests` may be
`null`. Audio output is `buffered_pcm`, `streaming_pcm`, or
`external_playback`. Cancellation is `none`, `playback_only`, or
`synthesis_and_playback`. Voice quality is `compact`, `enhanced`, or `premium`.
ACSS dimensions are `rate`, `average_pitch`, `pitch_range`, `stress`,
`richness`, and `volume`; post-synthesis dimensions are `gain`, `low_pass`,
`high_pass`, `pan`, `reverb`, and `echo`.

## Evolution Rules

- New request and response types may be added to version 1 when old clients can
  safely ignore the advertised feature.
- A breaking field or semantic change requires a new protocol version.
- Clients must ignore unknown capability feature strings.
- Omnivox must keep payload bounds, structured terminal errors, and legacy
  command behavior across protocol versions.
