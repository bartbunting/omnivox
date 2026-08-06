# Omnivox Control Protocol

Omnivox retains the Emacspeak line protocol for speech presentation and adds a
separate versioned channel for discovery, configuration, and diagnostics. The
control channel uses Base64-encoded UTF-8 JSON so native engine and voice IDs do
not need Tcl escaping and remain separate structured fields.

## Compatibility Boundary

- Existing clients and legacy commands continue to work unchanged.
- `emacsvox_tx` is an optional presentation-transaction frame. It is not used
  for persistent logical-voice configuration or discovery.
- A client must successfully request capabilities before using later control
  extensions.
- The server advertises only features it currently implements. Version 1
  currently implements capability negotiation, active-engine inventory,
  atomic logical-voice registration, queued logical-voice routing, and bounded
  replaceable presentation transactions. It also advertises tracked playback
  completion for clients that need a terminal result after queued audio ends.

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
  "server_version": "1.3.0",
  "supported_protocol_versions": [1],
  "features": [
    "control_v1",
    "engine_inventory",
    "emacsvox_tx",
    "legacy_commands",
    "logical_voice_registration",
    "logical_voice_routing",
    "preferred_engine",
    "stable_voice_ids",
    "tracked_playback_completion"
  ]
}
```

The request ID lets Emacsvox distinguish simultaneous main and notification
process responses. Server events are never injected into synthesized speech.

An inventory response contains an `inventory_generation`, the stable
`preferred_engine_id`, and an `engines` array. Each engine includes its stable
ID, runtime availability and health, capabilities, discovered physical voices,
and default voice ID. On Windows the server eagerly registers WinRT and eSpeak;
WinRT is preferred by default and eSpeak is retained as a fallback.
`OMNIVOX_ENGINE=espeak` reverses that preference without removing WinRT from
inventory. Other platforms currently register the compatibility-selected
engine. Descriptors are snapshotted before the reader loop starts, so inventory
requests cannot block stop commands behind synchronous synthesis. Registry
inventory is sorted by stable engine ID, and its generation advances when
engines or their descriptor state change. Persistent runtime circuit state is
overlaid on those snapshots; inventory generation also advances when an engine
fails, becomes probe-ready, starts probing, or recovers.

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
ACSS record. Supported rate and volume values map directly to `TtsSettings`;
average pitch interpolates the same ten 0.5-through-2.0 pitch multipliers used
by the Emacsvox adapter. Unsupported dimensions are omitted for that engine
without preventing speech. A runtime fallback recomputes this application for
the replacement engine instead of reusing the failed engine's capabilities.
Pitch range, stress, and richness remain registered and diagnosable, but the
current minimal `TtsSettings` interface cannot deliver them to an engine yet.

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

Emacsvox uses this extension only after capability negotiation and currently
frames replaceable aural presentations. It keeps sending the ordinary legacy
protocol to older servers. Frame validation failures are server diagnostics,
not control-channel response records.

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

Definitions and fallback policy are snapshotted when a batch is dispatched.
The worker resolves them against its current health-adjusted inventory before
speaking the batch, so an engine failure or recovery that happened while the
batch waited in the worker queue is respected. A registration received later
still affects only later batches. An unknown, unresolved, or
no-longer-registered route degrades to the preferred legacy engine and voice
instead of dropping speech. Hard stop and reset requests cancel playback and
request cancellation from every registered engine.

This first routing slice applies to queued `q`/`c` speech. Immediate `tts_say`
and `l` commands retain legacy preferred-engine behavior. For a queued logical
route, a runtime `VoiceNotFound` excludes that physical voice and an unavailable
or failed synthesis call excludes that engine from the dispatched batch's
inventory snapshot. Omnivox then re-runs the registered definition and fallback
policy and retries the identical text chunk on the newly resolved route.

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
messages. Exact physical identities are represented as separate fields:

```json
{
  "kind": "exact",
  "engine_id": "winrt",
  "voice_id": "winrt:HKEY_LOCAL_MACHINE\\SOFTWARE\\...\\David"
}
```

No consumer should construct or parse a combined `engine:voice` string.
Display names are presentation only and must not be used as stable identity.

## Evolution Rules

- New request and response types may be added to version 1 when old clients can
  safely ignore the advertised feature.
- A breaking field or semantic change requires a new protocol version.
- Clients must ignore unknown capability feature strings.
- Omnivox must keep payload bounds, structured terminal errors, and legacy
  command behavior across protocol versions.
