# Omnivox Engine Helper Protocol

Omnivox uses this protocol to host speech engines that cannot be loaded into
the main process. Its first consumers are the 32-bit Windows Eloquence and
DECtalk capture helpers built by Emacsvox. The protocol is engine-neutral so
the main process keeps voice routing, fallback, effects, mixing, playback
completion, and runtime health policy.

## Transport and Compatibility

Version 1 is a bidirectional stream of newline-terminated UTF-8 JSON objects.
The helper reserves standard output for protocol frames and writes diagnostics
only to standard error. Each frame contains `protocol_version`, a tagged
`type`, and normally a positive `request_id`; only an error caused before an
input request ID can be trusted may omit it. A frame is limited to 1 MiB.

The host starts every session with `hello` and supplies the versions it
supports:

```json
{"protocol_version":1,"request_id":1,"type":"hello","supported_protocol_versions":[1]}
```

The helper selects a common version and describes its implementation:

```json
{"protocol_version":1,"request_id":1,"type":"hello","selected_protocol_version":1,"helper_name":"Eloquence x86 helper","helper_version":"0.1.0"}
```

No inventory or synthesis request is valid until this exchange succeeds.
Unknown types or fields added by a later incompatible contract require a new
protocol version; helpers must not guess at incompatible semantics.

## Requests

Version 1 defines these request types:

- `hello`: negotiate a protocol version;
- `describe`: return the complete structured `EngineDescriptor`;
- `synthesize`: capture one utterance as PCM using an optional physical voice
  ID and normalized rate, pitch, and volume;
- `cancel`: request cancellation of a target synthesis request;
- `ping`: check whether the helper protocol loop is responsive;
- `shutdown`: request orderly helper termination.

Synthesis text is limited to 256 KiB. Rate and volume use the inclusive
zero-to-one range; pitch uses 0.5 through 2.0. A missing voice selects the
helper's advertised default. The helper reports the actual physical voice in
its `synthesis_started` response.

## Synthesis Responses

One accepted `synthesize` request produces this ordered sequence:

1. exactly one `synthesis_started` with sample rate, channel count, signed
   16-bit little-endian PCM format, and actual physical voice ID;
2. zero or more monotonically sequenced `audio_chunk` responses;
3. zero or more `markers` responses;
4. exactly one terminal `synthesis_completed`, `synthesis_cancelled`, or
   `error` response.

PCM chunks are Base64 encoded once in their JSON field and decode to at most
256 KiB. Each chunk contains complete interleaved audio frames.
`synthesis_completed` carries the total audio frame count so the host can
reject missing, repeated, or truncated chunks. A marker identifies a word,
sentence, phoneme, or native engine index using an audio-frame offset and
optional source-text range/value. Text ranges are byte offsets into the request
UTF-8; a helper leaves them absent when its native indexes cannot be mapped
truthfully. One synthesis carries at most 4096 markers across all responses.

Version 1 permits one active synthesis per helper because the initial native
engines are serialized. The helper must continue reading commands while its
native synthesis worker runs so `cancel`, `ping`, and `shutdown` do not wait
behind synthesis. A `cancel` request receives `cancel_accepted`; the target
request independently ends with `synthesis_cancelled`. Cancellation is not
reported as successful completion.

## Inventory and Errors

`describe` returns the same structured engine, capability, voice, availability,
and health contract exposed by Omnivox control inventory. The host validates it
before registration and never combines engine and native voice IDs into one
opaque identifier.

Errors carry a stable code, bounded human-readable message, and `retryable`
flag. Version 1 codes cover malformed requests, unsupported versions, oversized
payloads, unavailable engines, missing voices, invalid parameters, busy
helpers, synthesis failures, and internal failures. An error may omit its
request ID only when malformed input prevented the helper from trusting that
ID. An error owned by a synthesis request is terminal for that synthesis.

## Bounds, Timeouts, and Recovery

The Rust codec enforces these bounds before accepting content:

- 1 MiB per JSON frame;
- 256 KiB of synthesis text;
- 256 KiB of decoded PCM per audio chunk;
- 128 MiB of decoded PCM per synthesis request;
- 4096 markers per synthesis request;
- 4096 discovered physical voices;
- 16 advertised protocol versions.

The host will apply configurable startup, ordinary-request, synthesis-idle, and
shutdown timeouts. EOF, malformed or oversized output, request-ID/order
violations, and timeouts make the helper unhealthy. Omnivox then terminates the
child, opens the existing engine circuit, routes speech through configured
fallbacks, and uses `prepare_recovery_probe` to start and negotiate a fresh
helper after cooldown. Proprietary DLLs remain user-supplied and outside the
Omnivox distribution.

The authoritative Rust wire types and bounded codec are in
`omnivox-tts/src/helper_protocol.rs`. The generic process client and synthesis
stream validator are in `omnivox-tts/src/helper_engine.rs`. They negotiate and
validate inventory, enforce exact requested voice realization, reject malformed
or incomplete PCM streams, issue cancellation independently of the serialized
synthesis call, and replace a failed child during a recovery probe. Marker
frames are validated but cannot yet leave the helper engine because the current
`TtsEngine` result type carries audio only.

The Eloquence and DECtalk adapters share one C# protocol host while retaining
separate native capture implementations and executables. Windows Omnivox
discovers either helper independently. End-to-end smoke tests have exercised real
capture, cancellation, mixed-engine routing, fallback, PCM canonicalization,
tracked playback completion, and process replacement after a helper crash.
