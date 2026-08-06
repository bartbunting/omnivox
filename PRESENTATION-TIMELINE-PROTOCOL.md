# Omnivox Presentation Timeline Protocol

This document specifies the capability-gated structured presentation command
used by Emacsvox and Omnivox. It complements the legacy line protocol and the
control protocol in [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md).

## Negotiation and Command

A client may send a structured timeline only after the server advertises
`presentation_timeline_v1`. Playback-bound action and degradation events use
`playback_marker_events_v2`; terminal completion uses the existing tracked
completion contract.

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

Version 1 has this shape:

```json
{
  "protocol_version": 1,
  "generation": 27,
  "dispatch_id": 91,
  "spans": [
    {
      "id": 1,
      "text": "Example",
      "logical_voice_id": "voice-annotate",
      "acss": {"rate": 0.6, "average_pitch": 0.4},
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

## Speech Spans

Each span contains nonempty UTF-8 text and may independently select a logical
voice. Omnivox resolves that logical voice against the dispatch's routing and
health snapshot, retries bounded runtime failures through its configured
fallbacks, and reports the engine and physical voice actually used.

`acss` contains normalized `0.0..1.0` engine-rendered dimensions: rate,
average pitch, pitch range, stress, richness, and volume. Unsupported values
do not prevent speech; they are omitted and reported as degradation.

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

Older servers continue to receive legacy commands. Emacsvox converts a frozen
Aural presentation only when every captured operation is representable; an
unmodelled operation keeps the whole presentation on the legacy path so
speech, icons, and state cannot be duplicated or reordered by partial
conversion.

A buffered engine without markers remains a useful TTS engine. It still
provides ordered speech, per-span voice routing and fallback, ACSS it supports,
whole-span post-synthesis effects, queue/span-boundary audio, cancellation,
and tracked completion. Only precision-dependent optional actions degrade.
Speech is never dropped merely because marker or effect metadata is absent.
