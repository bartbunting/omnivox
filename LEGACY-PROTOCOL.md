# Legacy Emacspeak line protocol

This document specifies the newline-delimited compatibility protocol accepted
by the Omnivox server on standard input. It covers the baseline Emacspeak
commands and points to the separate specifications for capability-gated
Omnivox and Emacsvox extensions.

## Transport and grammar

Omnivox reads UTF-8 records terminated by LF or CRLF. Each logical record is
limited to 512 KiB, excluding the line terminator. An oversized or invalid
UTF-8 record is drained and rejected without terminating the server; parsing
resumes at the next record.

Commands use lowercase ASCII identifiers and one of these forms:

```text
command
command arguments
command {arguments}
```

Leading and trailing whitespace around a record is discarded. In the braced
form, the one outer pair of braces is removed and the enclosed text is retained.
Braces do not provide multiline framing: every command must still fit in one
input record. Empty and malformed records are ignored after an error is written
to standard error.

Baseline commands do not send acknowledgements to standard output. Invalid or
missing direct-command arguments generally leave state unchanged; some write a
diagnostic warning, but no legacy error record is guaranteed. Capability-gated
extensions use explicit event records as described in their specifications.

## Queue and dispatch model

`q`, `c`, `t`, `sh`, and `a` append items to one pending batch. `d` atomically
moves a valid nonempty batch to the bounded synthesis worker. Items retain wire
order, but their audio placement differs: speech and silence advance the main
speech timeline, tones use the independent tone stream, and audio icons overlay
through the sound stream.

The pending batch is limited to 4,096 items and 16 MiB of text/path payload. If
either limit is exceeded, the entire pending batch is poisoned and later items
are ignored until `d`, `s`, or `tts_reset`; `d` then rejects the whole batch
rather than speaking a prefix. Worker admission is also bounded and
nonblocking, so clients must not treat a completed write as proof of synthesis
or playback.

Process state is captured when a batch is dispatched, not when its first item
is queued. Inline `c` codes then modify only that dispatched batch and affect
subsequent items within it.

| Command | Arguments | Effect |
|---|---|---|
| `q` | text | Queue speech text. The braced form preserves surrounding whitespace inside the braces. |
| `c` | inline codes | Queue batch-local voice, logical-voice, or pitch changes described below. |
| `t` | `FREQUENCY_HZ DURATION_MS` | Queue an independent tone. Frequency must be finite and greater than zero through 24,000 Hz; duration is 1 through 60,000 ms. |
| `sh` | `DURATION_MS` | Queue silence on the speech timeline. The value must fit an unsigned 32-bit integer. |
| `a` | path | Queue a WAV or OGG audio icon on the sound stream. |
| `d` | none | Dispatch the pending batch. An empty dispatch is a no-op. |

For `a` and `p`, an unquoted path is used literally except that a leading `~/`
is expanded. A double-quoted Tcl word may encode spaces and the supported
escapes `\\`, `\"`, `\$`, `\[`, `\n`, `\r`, `\t`, and `\uNNNN`. A brace-delimited
argument has already had its outer braces removed and is otherwise literal.
Audio resources are limited to 16 MiB encoded and 30 seconds decoded.

### Inline codes

One `c` item can contain the following recognized tags. Copy physical voice IDs
from `omnivox --list-voices`; do not infer an ID from a display name.

```text
[{voice PHYSICAL_VOICE_ID}]
[[logical_voice LOGICAL_VOICE_ID]]
[[pitch FLOAT]]
```

A logical voice ID is 1 through 128 ASCII letters, digits, dots, underscores,
or hyphens and must have been registered through the control protocol. An
unknown logical voice falls back to the preferred legacy route. If multiple
recognized tags of the same kind occur in one code item, only the first match
is used.

## Immediate and lifecycle commands

| Command | Arguments | Effect |
|---|---|---|
| `s` | none | Hard stop: advance the generation, stop all audio streams and registered engines, cancel older queued synthesis, and clear the pending batch. |
| `tts_say` | text | Interrupt older speech and speak one string immediately. Tone and sound streams are not stopped. |
| `l` | text | Interrupt older speech and speak the supplied letter/text using character-rate and capitalization handling. |
| `p` | path | Play a WAV or OGG file immediately on the sound stream without dispatching or clearing the pending batch. |
| `version` | none | Speak the Omnivox version; it is not printed as a legacy response. |
| `tts_exit` | none | Exit the process successfully. |

`tts_say` and `l` use the current global engine preference rather than a named
logical voice. Their generation change suppresses stale synthesis results, but
only `s` asks every registered engine to stop its native call.

## Persistent state commands

These commands update the process state used by later immediate requests and
dispatches. Unless noted otherwise, a malformed value is ignored and the old
value remains in effect.

| Command | Arguments | Effect |
|---|---|---|
| `tts_set_punctuations` | `none`, `some`, or `all` | Select punctuation expansion. |
| `tts_set_speech_rate` | number | Values greater than 1 are divided by 100, then the normalized rate is clamped to `0.0..2.0`; for example, `60` becomes `0.6`. |
| `tts_set_character_scale` | float | Set the multiplier applied to the current speech rate for `l`. |
| `tts_split_caps` | flag | Enable CamelCase splitting only when the argument is exactly `1`; other direct values disable it. |
| `tts_set_capitalization_presentation` | `none`, `spoken`, `tone`, `spoken-tone`, or `custom` | Select legacy isolated-capital presentation. Structured clients supply explicit timeline actions instead. |
| `tts_set_voice` | physical voice ID | Set the startup-engine voice used by the legacy route. Use an exact ID from `--list-voices`. |
| `tts_set_pitch_multiplier` | float | Set the pitch multiplier; adapters conventionally use `0.5..2.0`. |
| `tts_set_voice_volume` | float | Set speech gain; adapters conventionally use `0.0..1.0`. |
| `tts_set_tone_volume` | float | Set tone gain; adapters conventionally use `0.0..1.0`. |
| `tts_set_sound_volume` | float | Set sound/icon gain; adapters conventionally use `0.0..1.0`. |
| `tts_set_speech_channel` | `left`, `right`, or `both` | Route the speech stream only. Process startup routing applies to all three streams. |
| `tts_reset` | none | Perform a hard stop, restore default state, and clear the pending batch. |

The direct float setters use Rust floating-point parsing without explicitly
rejecting non-finite values or enforcing every conventional adapter range at
this layer. Callers should remain inside the ranges above; a backend or audio
stage may clamp or reject other values.

`tts_sync_state PUNCTUATION SPLIT_CAPS LEGACY_CAPS RATE` updates punctuation,
split-caps state, and speech rate in one record. At least four fields are
required. `LEGACY_CAPS` is retained as a compatibility field but is currently
ignored; capitalization presentation is synchronized separately. Framed
transactions require exactly four valid fields, with both flags represented as
`0` or `1`.

## Deprecated commands

The following identifiers remain parseable so migration failures are explicit,
but they do not mutate state:

- `set_lang`
- `set_next_lang`
- `set_previous_lang`
- `set_preferred_lang`
- `tts_set_notification_channel`

Language selection now belongs to registered logical voices. Notification
routing uses a separate Omnivox process with its own `OMNIVOX_AUDIO_TARGET`.
Each deprecated command produces a warning and a structured
`unsupported_operation` control event.

## Capability-gated line commands

The following records share the line transport but have separate versioned
contracts:

| Command | Specification |
|---|---|
| `omnivox_control` | [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) |
| `emacsvox_tx` | [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) |
| `emacsvox_tracked_dispatch` | [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) |
| `emacsvox_marker_dispatch` | [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) |
| `emacsvox_tone` | [CONTROL-PROTOCOL.md](CONTROL-PROTOCOL.md) |
| `emacsvox_timeline` | [PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md) |
| `emacsvox_timeline_part` | [PRESENTATION-TIMELINE-PROTOCOL.md](PRESENTATION-TIMELINE-PROTOCOL.md) |

Clients must negotiate the relevant capability before sending an extension.
The maximum line and UTF-8 rules in this document still apply to its outer
record.
