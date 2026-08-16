# Omnivox Presentation Timeline Protocol

This document specifies the capability-gated structured presentation command
used by Emacsvox and Omnivox. It complements the legacy line protocol and the
control protocol in [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md).

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

The complete envelope is decoded, bounded, cross-referenced, and validated
before it can affect playback. File resources are loaded into immutable shared
PCM before the first span is submitted; generated tones and silence remain
validated recipes until their bounded render window. A bad field,
missing/reordered fragment, timeout, or unavailable resource rejects the whole
logical submission; the server does not play a valid prefix. Once a valid
part-zero header has been accepted, assembly failure reports `failed`; a stop
between parts reports `cancelled`. A complete aggregate whose generation is
already stale also reports `cancelled`. Each case retires the declared
generation.

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

`generation` participates in the same stale-work and stop-barrier policy as
framed legacy transactions. `dispatch_id` identifies marker-v2 and terminal
records. Span and action IDs are bounded and unique in their respective
namespaces. Version 3 permits at most 262,144 spans and 262,144 actions inside
the 16 MiB aggregate. Logical voice, action, replacement, and effect-state IDs
are at most 128 UTF-8 bytes; audio paths are at most 4096 UTF-8 bytes.

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

Receipt of a valid newer replaceable timeline advances cancellation for its
exact `(protocol_version, replacement_key)` domain immediately, before the
reader coalescing window expires. Only admission of the selected synthesis
request is delayed. The domain token follows synthesis, rendered windows,
deferred overlays, playback cues, and tracked completion. It removes queued or
unstarted tagged audio immediately and fades already active speech over three
milliseconds. Ordered, urgent, legacy, and differently keyed sources on the
same speech stream are not cleared. Unreached markers, semantic events, and
effect/overlay tails owned by the superseded timeline are cancelled.

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

`rate_offset`, when present, is a signed integer from `-20` through `20`. It
adjusts the server's current speech rate by that many points on its `0..100`
rate scale before engine-specific conversion: at a current rate of 75, `-1`
means 74 and `4` means 79. It does not change the current global rate. A span
must not contain both `rate_offset` and the absolute `acss.rate`. An offset of
zero is neutral and clients should normally omit it.

`effects` is one complete-state operation:

- `retain` continues the previous post-synthesis state;
- `replace` supplies a bounded state ID and complete normalized style;
- `end` returns to dry speech.

The initial state is dry. Effect style may contain gain, low-pass, high-pass,
pan, reverb, and echo. State continues across chunks, logical and physical
voice changes, and engine fallback until replaced or ended. Effects requiring
PCM degrade on an external-playback engine without causing text loss.

## Positions and Lifecycle

Physical placement and semantic lifecycle are separate fields.

A position is either a `span_boundary` or a validated UTF-8 `text_offset` in a
named span. Both forms declare `before` or `after` affinity. A lifecycle anchor
is independently `object`, `run`, or `transition`; it allows Emacsvox to retain
the richer meaning of an action without asking Omnivox to understand Emacs
modes, faces, rules, or schemes.

Text preprocessing preserves a source map across punctuation expansion,
split-capital handling, lowercasing, and chunking. Engines may resolve an
in-span request exactly, to a word boundary, to a span boundary, or omit only
the optional action. Timers are not used to guess playback position.

## Actions

Actions are applied in input order when several resolve to the same frame.

- `audio` validates and preloads a bounded file into immutable shared PCM.
  `mode` is `insert` or `overlay`; normalized
  volume and pan are independent of the speech effect state. `effect_bus` is
  `dry` or `speech`.
- `tone` has frequency and duration plus the same mode, volume, pan, and effect
  bus as an audio resource. Validated tones are generated only for their
  bounded render window.
- `silence` inserts a bounded duration, is materialized with its render window,
  and advances the primary clock.
- `semantic_event` has zero duration and is emitted only if playback consumes
  its resolved boundary.

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

Events are ordered playback cues, flushed before the dispatch's terminal
`completed`, `cancelled`, or `failed` record. The opaque semantic ID is data,
not executable content. Emacsvox keeps its richer Lisp value in a local table
and rejoins it only when the corresponding event arrives.

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
