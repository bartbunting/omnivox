# Omnivox Engine Helper Protocol

Omnivox uses this protocol to host speech engines that cannot be loaded into
the main process. Its first consumers are the 32-bit Windows Eloquence and
DECtalk capture helpers built by Emacsvox. The protocol is engine-neutral so
the main process keeps voice routing, fallback, effects, mixing, playback
completion, and runtime health policy.

## Transport and Compatibility

Versions 1 through 3 use a bidirectional stream of newline-terminated UTF-8 JSON objects.
The helper reserves standard output for protocol frames and writes diagnostics
only to standard error. Each frame contains `protocol_version`, a tagged
`type`, and normally a positive `request_id`; only an error caused before an
input request ID can be trusted may omit it. A frame is limited to 1 MiB.

The host starts every session with `hello` and supplies the versions it
supports:

```json
{"protocol_version":3,"request_id":1,"type":"hello","supported_protocol_versions":[3,2,1]}
```

The helper selects a common version and describes its implementation:

```json
{"protocol_version":3,"request_id":1,"type":"hello","selected_protocol_version":3,"helper_name":"Eloquence x86 helper","helper_version":"0.1.0"}
```

No inventory or synthesis request is valid until this exchange succeeds.
Unknown types or fields added by a later incompatible contract require a new
protocol version; helpers must not guess at incompatible semantics.

Omnivox offers version 3 first and retries each older supported envelope after
an `unsupported_version` response. Every later frame uses the selected version.
Versions 1 and 2 remain byte-compatible with their original contracts.

## Requests

All versions define these request types:

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

Version 2 adds a required `anchors` array to `synthesize`. Each entry carries
a unique non-empty opaque ID of at most 128 UTF-8 bytes, a `text_offset` on a
UTF-8 boundary in the exact request text, and `before` or `after` affinity. One
request carries at most 4096 anchors. The descriptor reports
`markers.requested_anchors` as `exact`, `word_boundary`, or `none`.

Version 3 adds optional `pitch_range`, `stress`, and `richness` synthesis
settings. Each uses the inclusive normalized zero-to-one ACSS range. Omnivox
only sends a value when the selected logical style supplies it and the helper
advertises support for that dimension; absence preserves the native voice's
default. Versions 1 and 2 omit these fields.

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

In version 2 an exact helper resolution is transported as a private
`requested_anchor` marker whose value is the opaque request ID. Omnivox removes
these from the ordinary marker list and returns them as structured resolved
anchors. Missing exact results can degrade through ordinary word markers; an
engine with no usable placement explicitly reports the anchor as omitted.
Resampling and silence trimming transform anchor frames alongside ordinary
marker frames.

All versions permit one active synthesis per helper because the initial native
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
flag. Error codes cover malformed requests, unsupported versions, oversized
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
- 4096 requested anchors per synthesis request and 128 bytes per anchor ID;
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
frames are validated, converted to common synthesis markers, and returned with
the helper's native PCM and realized physical voice. Their frame offsets follow
sample-rate conversion into canonical Omnivox audio. The Eloquence adapter now
merges bounded requested anchors with its Unicode word indexes, inserts both
through ECI's index API without splitting synthesis, and emits each native
callback frame. Before/after anchors at a shared source position retain
deterministic insertion order. It also inserts indexes at conservative source
sentence boundaries. Protocol v3 maps pitch range to ECI `vf`, stress to `vr`,
and richness to paired `vy` breathiness and `vv` compensation while preserving
independent normalized volume. The DECtalk adapter captures its native
phoneme-change and inserted-index records at DECtalk's utterance-relative sample
positions. It emits their numeric engine values without source ranges because
the native records do not identify request-text spans. For words, it inserts
collision-avoiding private indexes outside balanced DECtalk command/phonetic
spans and maps those callbacks to bounded UTF-8 source ranges. Existing caller
indexes remain distinct `native_index` markers, and words crossing native spans
are conservatively left unmarked. The same private-index path times conservative
source sentence boundaries. DECtalk rate and average pitch use native controls,
while protocol v3 maps pitch range to `pr/as`, stress to `hr/sr/qu/bf`, and
richness to `ri/sm`. Normalized volume scales the captured PCM because DECtalk's
speech-to-memory output is not affected by its native playback-volume command.

The Eloquence and DECtalk adapters share one C# protocol host while retaining
separate native capture implementations and executables. Windows Omnivox
discovers either helper independently. End-to-end smoke tests have exercised real
capture, cancellation, mixed-engine routing, fallback, PCM canonicalization,
Eloquence exact requested anchors plus word/sentence markers, DECtalk
word/sentence/phoneme/native-index markers, control mapping, tracked playback
completion, and process replacement after a helper crash.
