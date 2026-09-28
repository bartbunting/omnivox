# ADR 0011: Capital pitch preferences

- Status: Accepted
- Accepted: 2026-09-28; the maintainer authorized a global capital pitch,
  per-engine overrides and an explicit off option, retaining absolute pitch.
- Extends: [Local speech preferences](0009-local-speech-preferences.md).
- Related: [Saved speech defaults](0010-saved-speech-defaults.md),
  [actual-attempt tuning](0006-voice-selection-and-customization.md).

## Context

Isolated uppercase letter navigation currently selects host pitch `1.5`,
independently of ordinary pitch and word/sentence capitalization presentation.
Engine pitch mappings differ, so one fixed cue need not suit every engine.
Selecting an override before routing would apply the wrong cue after fallback.

## Decision

Add `speech.capital_pitch` to unreleased configuration version 2. It contains
an optional `default` and optional `engines` map. Values are a finite number
from `0.5` through `2.0`, or the exact string `"off"`. The default remains
`1.5`; omitted engine entries inherit it. Off retains ordinary pitch. Numeric
values replace host pitch, rather than multiplying the current pitch or
promising a particular acoustic change.

Require canonical IDs belonging to shipped or valid externally registered
engines, with at most 64 overrides. An override does not enable an engine,
change its capabilities or authorize its use. Keep ordinary adapter limits.

Capture the complete policy with engine startup. Apply it independently to
each actual isolated-capital synthesis attempt, including buffered/streaming
fallback, without mutating session pitch. Lowercase letter handling and
structured word/sentence capitalization actions retain existing behavior.
Reset and recovery retain the captured policy without rereading files.

New private startup schema 4 requires the complete policy. Schemas 1–3 imply
the historical fixed cue and retain their original serialized shape. The
public helper and speech protocols are unchanged.

## Consequences and alternatives

Users can adjust the cue across engines and override only exceptions. Per-voice
overrides, runtime cue commands and a new capitalization presentation system
remain separate work. A relative pitch increase would change existing behavior
when ordinary pitch differs from `1.0`; that is not this change.

Tests must exercise actual letter requests, inherited and overridden values,
off, Unicode, both forms of synthesis and fallback, reset and frozen recovery.
Functional pitch assertions do not establish audible cue strength. Exact
configuration rules belong to the
[reference](../reference/engine-configuration.md#capital-letter-pitch).
