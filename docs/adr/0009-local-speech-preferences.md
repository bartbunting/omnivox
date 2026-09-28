# ADR 0009: Local speech preferences

- Status: Accepted
- Accepted: 2026-09-28; maintainer authorized configurable chunk size and a
  second pass over useful host settings. Implementation and qualification remain
  separate from acceptance.
- Extends: [Engine registration](0008-extensible-engine-registration.md).
- Related: [Progressive audio and markers](0003-progressive-audio-and-markers.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md).

## Context

The fixed 15-word synthesis window is a trade-off between responsiveness,
per-call overhead and speech phrasing. Engines and reading workloads differ.
Local engine registration already has strict configuration and frozen startup
ownership; a separate source of speech settings would risk disagreement between
workers and recovery attempts.

## Decision

Extend the main configuration with version 2 and an optional `speech` object.
Its first setting is `max_chunk_words`, an integer from 1 through 100, defaulting
to 15. Version 1 retains its exact accepted fields and defaults. Helper manifests
remain version 1. Invalid settings fail startup; never silently ignore them or
interpret zero as unlimited synthesis.

Apply one host limit before engine selection to queued, immediate, preview and
structured speech. Preserve sentence/clause preference and UTF-8 source mapping.
Fallback must reuse the same prepared chunk. A client speech reset retains this
host preference. Direct diagnostic WAV synthesis retains its existing whole-text
operation and is not a chunking benchmark.

Freeze the resolved value with engine startup settings, shared across both lanes
and retained during recovery. New private snapshots use schema 2 and require the
complete speech settings. Historical schema-1 snapshots mean 15 words and retain
their original schema when serialized again, so an owner can still restart its
pinned older executable. Reading a snapshot never rereads local configuration.
The existing activation acknowledgement and helper/speech protocols are unchanged.

The upper bound is a conservative configuration guard, not a claim that 100 words
is optimal. Word count does not replace byte, PCM, timeline-action, cancellation
or ownership limits. Requested silence remains capped at 15 seconds. Playback
reserve and native rate calibration retain their accepted decisions.

## Consequences

Users may trade more frequent synthesis boundaries for longer phrases without
rebuilding Omnivox. Smaller windows add calls and may make speech sound uneven;
larger windows may delay the first result or reach an existing per-window action
limit. Keep the default unchanged and distinguish source timing from listening
or physical sound measurements. Native and audible qualification must be stated
explicitly; a passing configuration test alone does not establish either.

The [configuration reference](../reference/engine-configuration.md#configuration-version-2)
defines the exact contract. The [roadmap](../ROADMAP.md#host-configuration-follow-up)
records the second-pass candidates and the boundaries that should remain fixed.
This decision does not expose every internal constant or add a settings UI,
live reload, named-device selection or a new remote configuration operation.
