# ADR 0012: Saved audio output choices

- Status: Accepted
- Accepted: 2026-09-28; the maintainer authorized the remaining discussed output
  settings alongside the capital-pitch work.
- Extends: [Saved speech defaults](0010-saved-speech-defaults.md), specifically
  its deferred process-output configuration and routing-reset behavior.
- Related: [Native PulseAudio](0005-native-pulseaudio-output.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md).
- Extended by: [Windows default output recovery](0013-windows-default-output-recovery.md),
  which adds live endpoint switching inside the selected Windows device backend.

## Context

The existing output method, channel and PulseAudio latency request can only be
selected through command-line arguments or launcher environment. Saving them
in the host file makes local setup easier. A common file must still allow the
foreground and notification launchers to choose different channels.

## Decision

Add optional `audio` to unreleased configuration version 2: `backend` selects
`device`, `pulse` or `null`; `target` selects `left`, `right` or `both`;
`pulse_latency_ms` requests an integer from 10 through 200. Omitted values keep
the existing defaults: `device`, `both`, 20 ms. Strict input validation applies
even to settings unused by the selected backend.

Existing command-line output choices override launcher environment, which
overrides the file. PulseAudio latency has its existing environment override;
no new CLI flag is needed. Resolve the latency once when constructing the three
streams and reuse it during native reconnect. Reject unsupported selected
backends, without silent output fallback. An explicit supported override can
replace a saved backend unavailable on the current platform.

Capture file values in private startup schema 5 with engine configuration.
Historical schemas 1–4 imply existing file defaults and retain their original
shape. Per-lane launch environment and arguments keep their existing ownership,
so recovery preserves effective output choices without rereading files.

Speech reset restores the effective startup channel, including launcher/CLI
overrides, across speech, tone and sound. Temporary client speech-channel changes
can still be made afterwards. The backend and its latency request remain
process-lifetime choices; reset neither reopens output nor edits configuration.

Server startup and `--check` honor these settings. `--dump-wav` uses channel
selection in its processed output without opening an output backend. `--play-wav`
uses the shared strict configuration reader and output choices without starting
speech engines. It routes the file's channels without applying speech effects.

## Consequences and alternatives

The file becomes a baseline while existing launchers retain control of each
lane. Resetting channels unconditionally to both would discard that control.
Reading files during reset or reconnect could unexpectedly change active output.

This adds no named-device selection, automatic device switching, configurable
playback reserve or new protocol fields. A requested PulseAudio buffer is not a
measured acoustic latency. Validate precedence, channel samples, reset, frozen
recovery and historical compatibility; retain native platform evidence separately
from fixture results. See the
[configuration reference](../reference/engine-configuration.md#audio-output-settings).
