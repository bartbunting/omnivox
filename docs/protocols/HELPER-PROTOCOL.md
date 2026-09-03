# Omnivox Engine Helper Protocol

Omnivox uses this protocol to host speech engines that cannot safely be loaded
into the main process. Its consumers include the 32-bit Windows Eloquence and
DECtalk capture helpers under `windows-helpers` and the cross-platform Piper
helper. Emacsvox's Windows release target builds the former from this Omnivox
source with pinned inputs and stages the resulting executables.
The protocol is engine-neutral so the main process keeps voice routing,
fallback, effects, mixing, playback completion, and runtime health policy.

## Transport and Compatibility

Versions 1 through 5 use a bidirectional stream of newline-terminated UTF-8
JSON objects. The helper reserves standard output for protocol frames and
writes diagnostics only to standard error. Each frame contains
`protocol_version`, a tagged `type`, and normally a positive `request_id`; only
an error caused before an input request ID can be trusted may omit it. A frame
is limited to 1 MiB.

The host starts every session with `hello` and supplies the versions it
supports:

```json
{"protocol_version":5,"request_id":1,"type":"hello","supported_protocol_versions":[5,4,3,2,1]}
```

The helper selects a common version and describes its implementation:

```json
{"protocol_version":5,"request_id":1,"type":"hello","selected_protocol_version":5,"helper_name":"Eloquence x86 helper","helper_version":"0.1.0"}
```

No inventory or synthesis request is valid until this exchange succeeds.
Unknown types or fields added by a later incompatible contract require a new
protocol version; helpers must not guess at incompatible semantics.

Omnivox offers version 5 first and retries each older supported envelope after
an `unsupported_version` response. Every later frame uses the selected version.
Versions 1 through 3 remain byte-compatible with their original contracts.

## Requests

All versions define these request types:

- `hello`: negotiate a protocol version;
- `describe`: return the complete structured `EngineDescriptor`;
- `synthesize`: capture one utterance as PCM using an optional physical voice
  ID and normalized rate, pitch, and volume;
- `cancel`: request cancellation of a target synthesis request;
- `ping`: check whether the helper protocol loop is responsive;
- `shutdown`: request orderly helper termination.

Synthesis text is limited to 256 KiB. Volume uses the inclusive zero-to-one
range; pitch uses 0.5 through 2.0. Rate uses zero-to-one in versions 1 through
3 and zero-to-two in version 4. A missing voice selects the helper's advertised
default. The helper reports the actual physical voice in its
`synthesis_started` response.

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

Version 4 extends normalized rate through 2.0 for engines with native headroom.
The host constrains requests negotiated with older helpers to their original
1.0 maximum. Each helper adapter remains responsible for clamping to a lower
native engine limit where necessary.

Version 5 allows an engine that advertises `streaming_pcm` to publish bounded
PCM while its native synthesis call is still active. It also permits marker
batches to interleave with audio chunks, subject to the ordering rules below.
For a `requested_anchor` marker, version 5 adds the optional `resolution`
field: `exact`, `word_boundary`, `span_boundary`, or `omitted`. An absent
field retains the versions 2 through 4 meaning of `exact`.
Buffered engines remain valid version 5 peers and retain the version 4 response
order.

The [validated version 5 synthesis fixture](../protocol-fixtures/helper-synthesize-request-v5.json)
shows every settings field and one requested anchor. Versions 1 and 2 must omit
the version 3 style fields; version 1 must also omit `anchors`, while versions 2
through 5 require that array even when it is empty.

## Synthesis Responses

In versions 1 through 4, one accepted `synthesize` request produces this ordered
sequence:

1. exactly one `synthesis_started` with sample rate, channel count, signed
   16-bit little-endian PCM format, and actual physical voice ID;
2. zero or more monotonically sequenced `audio_chunk` responses;
3. zero or more `markers` responses;
4. exactly one terminal `synthesis_completed`, `synthesis_cancelled`, or
   `error` response.

In version 5, a `streaming_pcm` helper may interleave non-empty marker batches
and monotonically sequenced audio chunks between the same start and terminal
frames. Each marker must be sent before the audio chunk that reaches its frame
offset, and its offset must not be earlier than the number of audio frames
already sent. Markers exactly at the terminal frame boundary may follow the
last audio chunk. This lets the consumer apply bounded backpressure without
allowing playback to overtake timing metadata. A `buffered_pcm` version 5
helper uses the versions 1 through 4 ordering.

The sample rate and channel count in `synthesis_started` define the frame clock
for every version 5 wire chunk, marker, and terminal count. A progressive
helper may send signed 16-bit mono or stereo PCM in that announced native
format. Omnivox feeds non-canonical progressive PCM through one continuous,
bounded sinc converter and rescales markers into the resulting 44.1 kHz stereo
playback clock. It never restarts conversion at an `audio_chunk` boundary.

The [validated progressive success-stream fixture](../protocol-fixtures/helper-synthesis-success-v5.jsonl)
contains one complete sequence. Its frames are independently deserialized and
validated by a repository test.

PCM chunks are Base64 encoded once in their JSON field and decode to at most
256 KiB. Each chunk contains at least one complete interleaved audio frame;
helpers omit `audio_chunk` entirely when a synthesis produces no audio. A
`markers` response similarly contains at least one marker, and helpers with no
markers omit that response type. Empty synthesis remains valid and proceeds
from `synthesis_started` directly to a terminal response whose frame count is
zero.
`synthesis_completed` carries the total audio frame count so the host can
reject missing, repeated, or truncated chunks. A marker identifies a word,
sentence, phoneme, or native engine index using an audio-frame offset and
optional source-text range/value. Text ranges are byte offsets into the request
UTF-8; a helper leaves them absent when its native indexes cannot be mapped
truthfully. One synthesis carries at most 4096 markers across all responses.

In version 2 an exact helper resolution is transported as a private
`requested_anchor` marker whose value is the opaque request ID. Version 5 may
attach the explicit resolution described above, allowing a progressive engine
to publish a truthful approximation without waiting for the complete marker
list. Omnivox removes these private markers from the ordinary marker list and
returns them as structured resolved anchors. The `frame_offset` of an
`omitted` result is ignored. Missing exact results from buffered older helpers
can still degrade through ordinary word markers; an engine with no usable
placement explicitly reports the anchor as omitted. Resampling and silence
trimming transform non-omitted anchor frames alongside ordinary marker frames.

All versions permit one active synthesis per helper because the initial native
engines are serialized. The helper must continue reading commands while its
native synthesis worker runs so `cancel`, `ping`, and `shutdown` do not wait
behind synthesis. A `cancel` request receives `cancel_accepted`; the target
request independently ends with `synthesis_cancelled`. Cancellation is not
reported as successful completion.

For example, request 4 cancels synthesis request 3 with these three independent
frames:

```json
{"protocol_version":4,"request_id":4,"type":"cancel","target_request_id":3}
{"protocol_version":4,"request_id":4,"type":"cancel_accepted","target_request_id":3}
{"protocol_version":4,"request_id":3,"type":"synthesis_cancelled"}
```

The acknowledgement belongs to the cancel request; the terminal response
belongs to the target synthesis request.

Piper has no native in-call cancellation. Its helper acknowledges cancellation
on the protocol thread and suppresses stale PCM if the call returns promptly.
If it does not, the host's 250 ms cancellation watchdog kills and reaps the
helper process; a later recovery starts a fresh model process. Piper native
code and dynamic libraries are therefore never loaded into the main server.

## Inventory and Errors

`describe` returns the same structured engine, capability, voice, availability,
and health contract exposed by Omnivox control inventory. The host validates it
before registration and never combines engine and native voice IDs into one
opaque identifier. Its `descriptor` field has the exact `EngineDescriptor`
shape demonstrated by the
[validated control inventory fixture](../protocol-fixtures/control-inventory-response.json).

`capabilities.text_repertoire` declares the source characters the helper can
encode without replacement: `unicode`, `windows_1252`, or `iso_8859_1`. This is
additive descriptor metadata in every protocol version. A missing field is
`unknown`, for which Omnivox assumes only ASCII is safe. The Eloquence and
DECtalk helpers advertise their actual single-byte native boundaries, and their
encoders use exception fallback as a final guard against silent `?`
substitution.

Errors carry a stable code, bounded human-readable message, and `retryable`
flag. Error codes cover malformed requests, unsupported versions, oversized
payloads, unavailable engines, missing voices, invalid parameters, busy
helpers, synthesis failures, and internal failures. An error may omit its
request ID only when malformed input prevented the helper from trusting that
ID. An error owned by a synthesis request is terminal for that synthesis.

A helper whose separately supplied native runtime is missing or rejected still
negotiates `hello` and remains responsive to `ping` and `shutdown`. Its
`describe` and `synthesize` requests return `not_available` with a bounded
runtime diagnostic. This lets the host distinguish an optional unavailable
engine from a helper that crashed or failed to implement the protocol.

```json
{"protocol_version":4,"request_id":3,"type":"error","code":"busy","message":"native engine is already synthesizing","retryable":true}
```

## Bounds, Timeouts, and Recovery

The Rust codec enforces these bounds before accepting content:

- 1 MiB per JSON frame;
- 256 KiB of synthesis text;
- 256 KiB of decoded PCM per audio chunk;
- 128 MiB of decoded PCM per synthesis request;
- 4096 markers per synthesis request;
- 4096 requested anchors per synthesis request and 128 bytes per anchor ID;
- 4096 discovered physical voices;
- 16 advertised protocol versions;
- 16 KiB for bounded helper, voice-ID, error, and marker-value strings;
- sample rates from 1 through 384,000 Hz and 1 through 8 channels.

The host applies configurable startup, ordinary-request, and synthesis-idle
timeouts plus a bounded cancellation watchdog. EOF, malformed or oversized
output, request-ID/order violations, and timeouts make the helper unhealthy.
Omnivox then terminates the child, opens the existing engine circuit, routes
speech through configured fallbacks, and uses `prepare_recovery_probe` to start
and negotiate a fresh helper after cooldown. Proprietary DLLs remain
user-supplied and outside the Omnivox distribution.

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
discovers either helper independently. Their source, build targets, and
source-contract tests are owned together under `windows-helpers`; final WSL
bundle orchestration remains in Emacsvox. End-to-end smoke tests have exercised real
capture, cancellation, mixed-engine routing, fallback, PCM canonicalization,
Eloquence exact requested anchors plus word/sentence markers, DECtalk
word/sentence/phoneme/native-index markers, control mapping, tracked playback
completion, and process replacement after a helper crash.

The engine-neutral Rust helper host and shared Windows C# host select
progressive delivery only when they negotiate version 5 with an engine
advertising `streaming_pcm`. The same helpers downgrade their advertised mode
to `buffered_pcm` and call the complete-result path for an older peer. Rust
adapters supply canonical engine windows directly. RuTTS converts successive
signed 8-bit 10 kHz native callbacks through one bounded stateful sinc converter
before emitting those windows, so its native callback boundaries do not restart
resampling. RHVoice and eSpeak also feed their callback PCM through continuous
conversion and use native SSML marks to keep exact requested anchors
progressive. Their generated-input source maps retain word and sentence ranges
in the original UTF-8 request. Flite publishes all native word-start markers
before its first continuously converted callback window, allowing supported
word-boundary anchors to remain progressive. Piper forwards native synthesis
chunks progressively but, like RuTTS, retains buffered marker-dependent
presentation because its API exposes no synchronization markers. The Windows
host instead encodes native Eloquence and DECtalk
callback PCM at 11.025 kHz mono; the Rust receiver continuously converts that
stream to 44.1 kHz stereo. DECtalk retains one 512-sample native block so its
occasionally late marker callback can still precede the corresponding audio.
Both native wire PCM and expanded canonical PCM remain subject to the ordinary
128 MiB synthesis limit.
