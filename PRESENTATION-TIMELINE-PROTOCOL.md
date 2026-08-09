# Omnivox Presentation Timeline Protocol

This document specifies the capability-gated structured presentation command
used by Emacsvox and Omnivox. It complements the legacy line protocol and the
control protocol in [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md).

## Negotiation and Command

A client may send a version 2 structured timeline only after the server
advertises `presentation_timeline_v2`. The server also advertises and accepts
version 1 for decoding compatibility. Playback-bound action and degradation
events use `playback_marker_events_v2`; terminal completion uses the existing
tracked completion contract.

The command is one newline-terminated record:

```text
emacsvox_timeline {BASE64_UTF8_JSON}
```

The decoded JSON is limited to 256 KiB. The complete envelope is decoded,
bounded, cross-referenced, and validated before it can affect playback. Audio
resources are also loaded and decoded before the first span is submitted. A
bad field or unavailable resource rejects the whole command; the server does
not play a valid prefix.

## Envelope

Version 2 has this shape:

```json
{
  "protocol_version": 2,
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
namespaces.

## Delivery Policy

`delivery_policy` is `ordered`, `replaceable`, or `urgent`. Ordered and urgent
envelopes omit `replacement_key` and are never coalesced with an adjacent
timeline. A replaceable envelope requires a nonempty replacement key of at
most 128 UTF-8 bytes. Omnivox may coalesce adjacent replaceable envelopes only
when both use version 2 and their keys are identical; the discarded dispatch
receives `cancelled` before the selected dispatch can complete.

Urgent interruption remains an explicit stop barrier sent immediately before
the urgent timeline. The policy field prevents a later adjacent timeline from
silently superseding that urgent work. A stop encountered while any timeline
is still in the coalescing window cancels that timeline and consumes its
generation.

Version 1 omits both delivery fields and retains its original interpretation:
every version 1 envelope is replaceable in one implicit domain. Version 1 and
version 2 envelopes never coalesce with one another.

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

- `audio` preloads a bounded file. `mode` is `insert` or `overlay`; normalized
  volume and pan are independent of the speech effect state. `effect_bus` is
  `dry` or `speech`.
- `tone` has frequency and duration plus the same mode, volume, pan, and effect
  bus as an audio resource.
- `silence` inserts a bounded duration and advances the primary clock.
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

Current Emacsvox requires the version 2 capability for Aural structured
delivery. A server advertising only version 1 is an installation mismatch and
must be upgraded; Emacsvox does not silently lower ordered or urgent Aural
semantics to the legacy protocol. Within a negotiated version 2 transaction,
an unmodelled operation still keeps the whole presentation on the explicit
legacy path so speech, icons, and state cannot be duplicated or reordered by
partial conversion.

A buffered engine without markers remains a useful TTS engine. It still
provides ordered speech, per-span voice routing and fallback, ACSS it supports,
whole-span post-synthesis effects, queue/span-boundary audio, cancellation,
and tracked completion. Only precision-dependent optional actions degrade.
Speech is never dropped merely because marker or effect metadata is absent.
