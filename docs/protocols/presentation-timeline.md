# Omnivox Presentation Timeline Protocol

This document specifies the capability-gated structured presentation command
used by Emacsvox and Omnivox. It complements the legacy line protocol and the
control protocol in [control.md](control.md).

## Negotiation and Commands

A current client may send a version 3 structured timeline only after the
server advertises `presentation_timeline_v3`. The server also advertises and
accepts versions 1 and 2 for decoding compatibility, but current Emacsvox
requires version 3. Playback-bound action and degradation events use
`playback_marker_events_v2`; terminal completion uses the existing tracked
completion contract.

A decoded JSON document of at most 256 KiB uses one newline-terminated record:

```text
emacsvox_timeline {BASE64_UTF8_JSON}
```

Larger version 3 documents use consecutive newline-terminated records:

```text
emacsvox_timeline_part 3 GENERATION DISPATCH_ID INDEX COUNT DECODED_BYTES BASE64_FRAGMENT
```

`INDEX` is zero-based. `COUNT` is from 1 through 64, the decoded aggregate is
at most 16 MiB, and every nonempty Base64 fragment is bounded to the encoded
size of one 256 KiB frame. All records repeat identical generation, dispatch,
count, and decoded-size fields. They must arrive in exact index order within
five seconds of the first part. Only the final fragment may contain Base64
padding.

Fragments are transport slices of one Base64 encoding, not independently
decoded envelopes. Omnivox concatenates them, checks the exact decoded byte
count, decodes the original JSON once, and requires the envelope generation
and dispatch ID to match the outer header. This preserves span identity,
UTF-8 offsets, action anchors, effect state, and one presentation clock across
transport boundaries.

The complete envelope is decoded, bounded, cross-referenced, and its prepared
action-window density is validated before synthesis-queue admission. After
admission, the worker loads every file resource into immutable shared PCM
before synthesizing the first new span; generated tones and silence remain
validated recipes until their bounded render window. A bad field,
missing/reordered fragment, timeout, or unavailable resource rejects the whole
logical submission, so the server does not play a valid prefix of the new
timeline. For replaceable delivery, successful queue admission may already
have cancelled the preceding domain owner before later resource preparation
fails. Once a valid part-zero header has been accepted, assembly failure
reports `failed`; a stop between parts reports `cancelled`. A complete
aggregate whose generation is already stale also reports `cancelled`. Each
case retires the declared generation.

## Palette-aware isolated characters

A server advertising `palette_letter_v1` accepts:

```text
emacsvox_letter {BASE64_UTF8_JSON}
```

The payload is a single direct-frame version 4 or 5 envelope using the same
registered voice, generation, dispatch and marker contracts as ordinary
layered timelines. It requires exactly one `layered` or `engine_layered` span
(the latter requires version 5), containing one Unicode scalar other than
whitespace or a control character. Delivery must be `ordered`, with no
replacement key and no actions. Multipart transport and legacy spans are not
accepted. Clients also negotiate the complete layered/native capability
bundle required by their chosen envelope.

The server validates the envelope and registry before interrupting older
speech. It leaves tones and sounds running, then snapshots routing for the
character. Each actual choice, including a fallback, composes its shared,
choice and context tuning over the character-rate base. Native parameters and
playback-bound voice receipts use the ordinary layered contracts. It does not
change the global voice or registry.

Like legacy `l`, this path lowercases the input (including Unicode lowercase
expansions), uses the actual engine's configured absolute uppercase pitch (or
preserves ordinary palette pitch when the cue is off), bypasses prose punctuation
and capitalization transforms, and retains the short
streaming startup reserve. An enabled capital cue supplies pitch context to
native parameter planning; an off cue preserves ordinary native tuning. Source
case is retained in the submitted envelope.
Invalid input is rejected before interruption; stale generations are cancelled.
Older servers continue to receive legacy `l` and cannot apply a palette voice
to that command.

## Envelope

Version 3 has the same semantic fields introduced by version 2 and this shape:

```json
{
  "protocol_version": 3,
  "generation": 27,
  "dispatch_id": 91,
  "delivery_policy": "replaceable",
  "replacement_key": "speaker",
  "spans": [
    {
      "id": 1,
      "text": "Example",
      "logical_voice_id": "voice-annotate",
      "acss": {"average_pitch": 0.4},
      "rate_offset": -4,
      "effects": {
        "mode": "replace",
        "state_id": "room.1",
        "style": {"pan": 0.25, "reverb": 0.2}
      }
    }
  ],
  "actions": [
    {
      "id": "opening-cue",
      "position": {
        "position": "span_boundary",
        "span_id": 1,
        "affinity": "before"
      },
      "lifecycle_anchor": "object",
      "type": "audio",
      "path": "C:/sounds/open.ogg",
      "mode": "overlay",
      "volume": 1.0,
      "pan": 0.5,
      "effect_bus": "dry"
    }
  ]
}
```

The complete [validated version 3 fixture](../protocol-fixtures/presentation-timeline-v3.json)
contains two spans, replace/end effect state, both position types, and all four
action types. A repository test deserializes it through the public Rust wire
type and runs the same semantic validation used by the server.

`generation` participates in the same stale-work and stop-barrier policy as
framed legacy transactions. `dispatch_id` identifies marker-v2 and terminal
records. Span and action IDs are bounded and unique in their respective
namespaces. Version 3 permits at most 262,144 spans and 4,096 actions inside
the 16 MiB aggregate; versions 1 and 2 permit at most 4,096 of each in their
direct frames. After punctuation and CamelCase preparation, each configured
speech window may own at most 512 combined client actions and internal
capitalization anchors. Logical voice, action, replacement, and effect-state
IDs are at most 128 UTF-8 bytes; audio paths are at most 4096 UTF-8 bytes.
The host's [chunk limit](../reference/engine-configuration.md#configuration-version-2)
defaults to 15 words and may be configured from 1 through 100. This local setting
does not change the wire format or raise the 512-action bound; admission and
synthesis use the same frozen limit.

## Delivery Policy

`delivery_policy` is `ordered`, `replaceable`, or `urgent`. Ordered and urgent
envelopes omit `replacement_key` and are never coalesced or evicted. A
replaceable envelope requires a nonempty replacement key of at most 128 UTF-8
bytes. Omnivox coalesces adjacent reader submissions, and the bounded worker
handoff atomically supersedes an older queued envelope, only when both use the
same policy-bearing protocol version and their keys are identical. Under
capacity pressure it may also cancel the oldest queued replaceable envelope
from another domain before rejecting incoming work. Every discarded dispatch
receives `cancelled`; a replacement that cannot itself be admitted leaves the
older queued request intact. Version 2 and version 3 envelopes do not coalesce
with one another.

Urgent interruption remains an explicit stop barrier sent immediately before
the urgent timeline. The policy field prevents later work from silently
superseding or evicting that urgent work. A stop encountered while any timeline
is still in the coalescing window cancels that timeline and consumes its
generation; an accepted stop also cancels all older requests still waiting in
the worker handoff.

After the reader selects a replaceable timeline from its bounded coalescing
window, successful synthesis-queue admission atomically activates cancellation
for the exact `(protocol_version, replacement_key)` domain. If admission fails,
the prior queued or active domain owner remains intact. Once activated, the
domain token follows synthesis, rendered windows, deferred overlays, playback
cues, and tracked completion. It removes queued or unstarted tagged audio
immediately and fades already active speech over three milliseconds. Ordered,
urgent, legacy, and differently keyed sources on the same speech stream are not
cleared. Unreached markers, semantic events, and effect/overlay tails owned by
the superseded timeline are cancelled.

Version 1 omits both delivery fields and retains its original interpretation:
every version 1 envelope is replaceable in one implicit domain. Version 1 does
not coalesce with either policy-bearing version.

The cross-repository UTF-8 interoperability fixtures use generation 27,
dispatch 91, one span containing `café 日本`, and no actions. Their unwrapped
Base64 payloads are:

```text
V1 eyJwcm90b2NvbF92ZXJzaW9uIjoxLCJnZW5lcmF0aW9uIjoyNywiZGlzcGF0Y2hfaWQiOjkxLCJzcGFucyI6W3siaWQiOjEsInRleHQiOiJjYWbDqSDml6XmnKwifV0sImFjdGlvbnMiOltdfQ==
V2 eyJwcm90b2NvbF92ZXJzaW9uIjoyLCJnZW5lcmF0aW9uIjoyNywiZGlzcGF0Y2hfaWQiOjkxLCJkZWxpdmVyeV9wb2xpY3kiOiJyZXBsYWNlYWJsZSIsInJlcGxhY2VtZW50X2tleSI6InNwZWFrZXIiLCJzcGFucyI6W3siaWQiOjEsInRleHQiOiJjYWbDqSDml6XmnKwifV0sImFjdGlvbnMiOltdfQ==
```

## Speech Spans

Each span contains nonempty UTF-8 text and may independently select a logical
voice. Omnivox resolves that logical voice against the dispatch's routing and
health snapshot, retries bounded runtime failures through its configured
fallbacks, and reports the engine and physical voice actually used.

`acss` contains normalized `0.0..1.0` engine-rendered dimensions: rate,
average pitch, pitch range, stress, richness, and volume. Unsupported values
do not prevent speech; they are omitted and reported as degradation.

`rate_offset`, when present, is a signed integer from `-20` through `20`.
Omnivox adds `rate_offset / 100` to the stored normalized host speech rate and
clamps the derived one-shot rate to the normalized ACSS `0.0..1.0` range before
engine-specific conversion. Expressed on the host's integer point scale, at a
stored rate of 75, `-1` means 74 and `4` means 79. An extended host rate above
100 remains at 100 unless a negative offset brings it below that ceiling. It
does not change the stored global rate. A span must not contain both
`rate_offset` and the absolute `acss.rate`. An offset of zero is neutral and
clients should normally omit it.

`effects` is one complete-state operation:

- `retain` continues the previous post-synthesis state;
- `replace` supplies a bounded state ID and complete normalized style;
- `end` returns to dry speech.

The initial state is dry. Effect style may contain gain, low-pass, high-pass,
pan, chorus, reverb, and echo. Chorus is duration-preserving; its normalized
amount controls a bounded modulated delayed copy without moving source markers.
State continues across chunks, logical and physical voice changes, and engine
fallback until replaced or ended. Effects requiring PCM degrade on an
external-playback engine without causing text loss.

## Positions and Lifecycle

Physical placement and semantic lifecycle are separate fields.

A position is either a `span_boundary` or a validated UTF-8 `text_offset` in a
named span. Both forms declare `before` or `after` affinity. A lifecycle anchor
is independently `object`, `run`, or `transition`; it allows Emacsvox to retain
the richer meaning of an action without asking Omnivox to understand Emacs
modes, faces, rules, or schemes.

Text preprocessing preserves a source map across punctuation expansion,
split-capital handling, and chunking. Engines may resolve an in-span request
exactly, to a word boundary, to a span boundary, or omit only the optional
action. Timers are not used to guess playback position.

## Actions

Actions are applied in input order when several resolve to the same frame.

- `audio` validates a path and, after admission, preloads a bounded file into
  immutable shared PCM. `mode` is `insert` or `overlay`; normalized volume and
  pan are independent of the speech effect state. `effect_bus` is `dry` or
  `speech`.
- `tone` has frequency and duration plus the same mode, volume, pan, and effect
  bus as an audio resource. Validated tones are generated only for their
  bounded render window.
- `silence` inserts a bounded duration, is materialized with its render window,
  and advances the primary clock.
- `semantic_event` has zero duration and is emitted only if playback consumes
  its resolved boundary.

An individual OGG/WAV file is limited to 16 MiB on disk and 30 seconds decoded.
A tone is limited to 24 kHz and 60 seconds; inserted silence must be from 1
through 15,000 ms. Longer requests are rejected, not shortened. Resource
preparation permits at most 64 MiB of retained canonical
`f32` PCM storage per presentation. Repeated references to one shared decoded
file count once, while predicted private channel/pan/effect copies, generated
tones, inserted silence, and effect tails count against the same budget.
A final post-synthesis reverb/echo tail is capped at four seconds. Chorus does
not add a final tail.
Independently, the renderer caps one prepared speech window at 120 seconds of
primary output. Because synthesized duration is not known at admission, an
overrun is a runtime failure and can occur after earlier windows have already
been queued.

Inserted audio shifts later output markers and semantic events. Overlay audio
does not advance the primary speech clock, may overlap following speech, and
still extends tracked completion through its tail. Stop and replacement cancel
all unreached actions, carried overlay/effect tails, and semantic events.

## Marker Protocol Version 2

Version 2 retains the version 1 utterance and engine marker records and adds:

- `semantic_event_reached`, containing the opaque action ID;
- `timeline_action_resolved`, reporting `exact`, `word_boundary`,
  `span_boundary`, or `omitted` placement;
- `timeline_style_degraded`, reporting ACSS or post-synthesis dimensions that
  were omitted after actual route/fallback selection.

Decoded version 2 records have these shapes:

```json
{"protocol_version":2,"dispatch_id":91,"sequence":5,"type":"semantic_event_reached","utterance_id":1,"action_id":"done-reached"}
{"protocol_version":2,"dispatch_id":91,"sequence":6,"type":"timeline_action_resolved","utterance_id":1,"action_id":"word-tone","resolution":"word_boundary"}
{"protocol_version":2,"dispatch_id":91,"sequence":7,"type":"timeline_style_degraded","utterance_id":1,"degraded_acss":["pitch_range"],"degraded_effects":["echo"]}
```

The [validated marker-event fixture](../protocol-fixtures/playback-marker-events-v2.jsonl)
contains the same records. Action resolution is exactly `exact`,
`word_boundary`, `span_boundary`, or `omitted`; degradation arrays use the ACSS
and post-synthesis dimension names defined by the control contract.

Events are ordered playback cues, flushed before the dispatch's terminal
`completed`, `cancelled`, or `failed` record. The opaque semantic ID is data,
not executable content. Emacsvox keeps its richer Lisp value in a local table
and rejoins it only when the corresponding event arrives. The common marker
reporter losslessly bounds outstanding marker-event output to 8,192 records and
16 MiB of serialized records, backpressuring event production rather than
dropping a reached cue.

## Compatibility and Degradation

Current Emacsvox requires the version 3 capability for Aural structured
delivery. A server advertising only version 1 or version 2 is an installation
mismatch and must be upgraded; Emacsvox does not silently lower ordered,
urgent, multipart, or insert-action semantics to the legacy protocol. Within a
negotiated version 3 transaction, an unmodelled operation still keeps the whole
presentation on the explicit legacy path so speech, icons, and state cannot be
duplicated or reordered by partial conversion. A resource or action that
cannot satisfy the declared bounds rejects the logical presentation before any
part is written rather than degrading it to legacy output.

A buffered engine without markers remains a useful TTS engine. It still
provides ordered speech, per-span voice routing and fallback, ACSS it supports,
whole-span post-synthesis effects, queue/span-boundary audio, cancellation,
and tracked completion. Only precision-dependent optional actions degrade.
Speech is never dropped merely because marker or effect metadata is absent.

## Mixed timelines, version 4

Version 4 preserves explicit `legacy` and `layered` span wrappers and carries
an admitted `registry_generation`. Shared settings, the actual selected choice
patch and the winning sparse context compose in that order; output placement
follows composition. The [paired wire contract](voice-choice-tuning.org)
defines the complete grammar and bounds. Clients negotiate all three bundle
features before using it. Old versions retain the rules above.

The existing single-document command and multipart command both accept version
4. Multipart headers and the decoded document must agree on version and identity;
no prefix enters synthesis. Registry references, text encoding and all action
windows are checked before replacement decisions. Replacement domains include
protocol version, so a version-4 key cannot cancel version-3 work. Consecutive
legacy spans retain effect state only within their legacy run; every such run
starts neutral. Playback uses marker version 3 and one tracked terminal record.


## Native timelines, version 5

Version 5 adds the `engine_layered` span mode and preserves the existing legacy
and layered forms. Each named span must match its registered definition mode
and the document's admitted registry generation. The
[engine parameter contract](engine-voice-parameters.md) defines native choice
composition, contextual masking and ordinary common-only degradation.

The single-document and multipart commands accept version 5 within the existing
bounds. Multipart declarations must match the decoded byte count and document
identity. New envelopes reject unknown and duplicate fields, including nested
legacy settings and action positions, without changing older readers.
Replacement domains remain version-local. Catalogue snapshots are frozen on
admission and charged to the queue's payload budget; execution does not query
helpers for metadata.

Playback uses marker version 4. Each non-legacy choice receipt requires a
nullable `native_application` beside `choice`: null means no native block was
requested, `common_only` states why the native block could not apply, and
`applied` contains the adapter's identity, masked controls and a connection-local
plan reference. The paired utterance-start and choice records are published
when the first PCM frame is consumed. Preparation, empty audio, cancellation
before consumption and failed pre-audio fallback attempts cannot publish them.
The existing receipt and remote-line limits apply before queuing PCM.

The server advertises native timelines with the complete native-parameter
bundle, including strict previews and explanation operations. Clients must
negotiate capabilities on each connection; a supported timeline version does
not imply that every available engine supplies qualified native controls.

## Richer tones, version 6

The `rich_tones_v1` bundle advertises standalone tone version 2 and timeline
version 6 together. Both render the same sound definition and apply per-cue
gain and the worker's master tone gain once each. Clients negotiate per worker,
including remote workers. Use version 6 consistently on a negotiated connection,
even for Classic cues or timelines without tones: replacement domains include
the protocol version. Version 6 cannot replace version 5 work with the same key.

Version 6 retains version 5's speech spans, registry checks, action positions,
effect buses, markers and native-choice evidence. Single-record and multipart
commands keep the existing byte, action, resource and generation bounds; the
multipart header uses version 6. A tone action may additionally contain:

```json
"shape": {
  "waveform": "triangle",
  "envelope": {
    "attack_ms": 1,
    "decay_ms": 25,
    "sustain_level": 0.25,
    "release_ms": 10
  }
}
```

`waveform` is required inside `shape` and is `sine`, `triangle` or `square`.
Omitting `shape` retains Classic sine samples and fades. Omitting `envelope`
retains Classic fades for the selected waveform. Explicit nulls, unknown fields,
duplicate JSON members, shapes on non-tone actions and invalid values reject
the whole document before admission. Versions 1–5 do not support shape fields.

Standalone commands use the existing presentation clock and queued dispatch:

```text
emacsvox_tone 2 MODE FREQUENCY DURATION GAIN WAVEFORM classic
emacsvox_tone 2 MODE FREQUENCY DURATION GAIN WAVEFORM adsr ATTACK DECAY SUSTAIN RELEASE
d
```

`MODE` is `insert` or `overlay`. Commands work directly or inside `emacsvox_tx`;
exact field counts are required. Version 1 and the ordinary independent `t`
beep are unchanged. The [shared fixture](../protocol-fixtures/richer-tones.json)
contains matching standalone and timeline definitions.

Duration is an integer from 1 through 60000 milliseconds. Each envelope stage
time is an integer from 0 through 60000 milliseconds. Gain and sustain level
are finite numbers from 0 through 1; sustain is a level, not a time. Frequency
is finite and greater than zero, at most 20000 Hz for a non-Classic shape.
Classic sine with no envelope, including gain-only changes, retains the older
24000 Hz limit. Stereo delay, channel routing and timeline pan retain their
existing behavior. Timeline action gain remains outside speech-bus effects.

Duration includes all envelope stages. When attack, decay and release exceed
it, their times shrink proportionally and sustain has no hold. Frame rounding
never extends duration. Zero-length stages jump to the following stage.
ADSR output also has a smooth endpoint cap of at most one millisecond or half
the tone, whichever is shorter; the level is the minimum of the cap and ADSR.
Both endpoints reach silence. Classic samples bypass this envelope path.
Triangle and square use at most 32 odd harmonics strictly below Nyquist, with
fixed amplitude bounds and no automatic perceived-loudness matching.

Tone generation checks hard-stop and keyed cancellation before allocation,
every 256 frames and before publishing PCM. Cancellation discards partial PCM
without dry-speech fallback or a failed-request status. Active-source stop
fades remain unchanged; envelope release never delays Stop. These implementation
checks do not establish acoustic or platform qualification; see the
[delivery plan](../plans/richer-tones.md).


## Extended tones, version 7

The `rich_tones_v2` bundle adds `presentation_tone_v3` and
`presentation_timeline_v7` to the complete earlier rich-tone bundle. Negotiate
all three per worker, including after reconnect. Use timeline 7 consistently
on that connection, even when no extended tone is present. Version 7 retains
the native speech, marker, registration, framing, bounds and cancellation
contracts of version 6; replacement remains local to its version and key.
Version 6 rejects extended shapes rather than silently dropping their fields.

Version 7 shape objects additionally accept:

- `waveform: "pulse"` with required finite `pulse_width` from 0.05 through 0.95.
  Width is the high portion of one cycle. At 0.5 it matches Square within
  floating-point precision. Width is rejected for other waveforms.
- `waveform: "sawtooth"`, with no width field.
- Optional finite `end_frequency_hz`, greater than zero through 20000 Hz.
  Frequency moves linearly from the action's starting frequency to this end
  over the fixed duration. Omission keeps constant pitch. The starting pitch
  also uses the shaped-tone limit of 20000 Hz.
- Optional `envelope_curve: "linear"` or `"exponential"`, only with an ADSR
  envelope. Omission retains linear stages. For stage fraction x, the
  exponential rise is `expm1(4*x)/expm1(4)`; decay and release use its reversed
  fraction, scaled between their endpoint levels. Endpoint smoothing and
  proportional stage shortening remain unchanged.

Explicit nulls, duplicate or unknown fields and invalid combinations reject
the entire request. Neither sweep nor envelope extends the total duration.
Pulse and Sawtooth use at most 64 harmonics below Nyquist. Sweeps select a
fixed harmonic set below Nyquist at the higher endpoint, avoiding harmonic
switching during playback. Pulse removes the DC component and uses a
width-dependent, frequency-independent coefficient bound; Sawtooth uses a
fixed coefficient bound. No perceived-loudness matching is implied.

Standalone version 3 appends exactly three fields to either version-2 form:

```text
emacsvox_tone 3 MODE FREQUENCY DURATION GAIN WAVEFORM classic WIDTH END CURVE
emacsvox_tone 3 MODE FREQUENCY DURATION GAIN WAVEFORM adsr ATTACK DECAY SUSTAIN RELEASE WIDTH END CURVE
```

Use `-` for each omitted extension, a number for WIDTH or END, and `linear`
or `exponential` for CURVE. A Classic envelope therefore requires CURVE `-`.
Standalone versions 1 and 2 retain their exact forms and reject these additions.
The [extended fixture](../protocol-fixtures/extended-tones.json) pairs the
standalone and timeline definitions. Both use the shared renderer and existing
stop behavior; a normal release never delays cancellation. Automated PCM and
transport checks are separate from listening and native platform acceptance.
