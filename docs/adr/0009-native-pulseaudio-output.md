# ADR 0009: Opt-in Native PulseAudio Output

- Status: Accepted
- Date: 2026-09-07

## Context

Linux under WSLg currently sends canonical PCM through Rodio/CPAL, the ALSA
PulseAudio plugin, and WSLg's PulseAudio/RDP output. Small ALSA buffer requests
stalled in the local experiment. A direct PulseAudio client can request smaller
buffers and control stream flushing and idle suspension explicitly. It still
uses the WSLg RDP transport, whose buffering is outside Omnivox's control.

## Decision

Add an opt-in `pulse` audio backend on Linux. Retain `device` as the default
and `null` for device-free testing. Dynamically bind the stable system
`libpulse.so.0` playback API; do not bundle a server, install packages, change
engine process boundaries, or alter helper or Emacsvox protocols. The existing
locked `libloading` dependency is also used by the Linux audio crate.

Each speech, tone, and sound lane owns a persistent PulseAudio connection and
stream, a source worker, and a native threaded mainloop. A blocked progressive
source cannot block another lane or native connection-state callbacks. The
existing canonical source wrappers retain routing, request cancellation,
markers, presentation effects, ordering, queue limits, and the three-window
progressive reserve required by ADR 0006. PulseAudio mixes the three streams
on its selected default sink; foreground and notification remain independent
Omnivox processes under ADR 0008.

Request 20 ms total latency with `PA_STREAM_ADJUST_LATENCY`, write at most
about 5 ms at once, and bound the requested maximum buffer. Permit an explicit
10–200 ms request through `OMNIVOX_PULSE_LATENCY_MS`. Buffer requests are hints;
log negotiated byte sizes, timing availability, latency estimates, and
underflow counters. Automatic prebuffering at one frame prevents a producer
gap from advancing the read cursor beyond future speech. Idle streams drain
then cork; a new source cancels a pending drain.

Before uncorking, prime two small writes (about 10 ms), or the available
shorter source/server buffer. This reserve sits within the existing latency
request and avoids playing the first packet dry during the uncork round trip.
Resume is asynchronous so feeding continues while its acknowledgement is in
flight. Corking and flushing for retirement remain acknowledged operations.
The WSL comparison launcher overrides the general 20 ms default with a 40 ms
request after repeated short-tone tests; the user can override that profile.

Stream-wide stop/backlog retirement discards unsubmitted PCM, corks and flushes
that lane, then permits its newer generation to start. It prioritizes immediate
retirement over the existing device backend's short stop fade. Selective keyed
cancellation retains the shared source's fade and never flushes unrelated
requests. PCM already consumed by WSLg/RDP cannot be recalled. Markers and
completion continue to describe source consumption, not physical sound.

Connection failure retires current and queued sources and rejects future
appends on that lane. Connection setup, stream operations, writable-buffer
stalls, and drains have deadlines. Backend shutdown also interrupts a stalled
progressive source. Automatic reconnection remains separate device-recovery
work; a failed trial is restarted explicitly.

## Consequences

The experiment bypasses ALSA without duplicating synthesis or timeline code.
It adds three native event threads and three source workers per Omnivox
process. Linux needs a compatible installed libpulse and an accessible server;
other platforms reject explicit `pulse` selection with an actionable error.
The existing default backend continues to work without libpulse.

Regression tests cover bounded writes, idle/drain transitions, immediate
replacement, cancellation domains, concurrent lanes, stalled progressive
sources, and device failure. Real WSLg acceptance remains necessary. Client
buffer estimates alone cannot establish acoustic onset, stop-to-silence, or
parity with Windows. A Windows PCM-output bridge is a possible later
experiment if the remaining RDP transport delay warrants it.
