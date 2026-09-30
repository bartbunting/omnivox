# ADR 0005: Opt-in native PulseAudio output

- Status: Accepted
- Consolidated: 2026-09-27 from the accepted native PulseAudio and recovery decision.
- Related: [Progressive playback](0003-progressive-audio-and-markers.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md).

## Context

The WSLg ALSA bridge adds buffering and limits direct control of flushing and
idle transitions. Native PulseAudio can control those operations but still uses
WSLg's RDP transport, whose behavior is outside Omnivox.

## Decision

Keep `device` as the default backend and offer explicit `pulse` selection on
Linux. Dynamically bind installed `libpulse.so.0` through the existing locked
loading dependency. Do not bundle a server, install packages or move synthesis
into the output backend. Other platforms reject `pulse` with a useful error.

Speech, tone and sound have separate persistent connections, streams, source
workers and native event threads. A blocked producer cannot block other lanes
or native callbacks. Use the common source wrappers, cancellation, marker,
effect and completion semantics. Foreground and notification remain independent
Omnivox processes. PulseAudio mixes each process's streams on its selected sink.

Request 20 ms with `PA_STREAM_ADJUST_LATENCY`, allow an explicit 10–200 ms
`OMNIVOX_PULSE_LATENCY_MS` request, and write approximately 5 ms at a time with
a bounded maximum buffer. These are requests; record negotiated values. The
WSLg comparison profile's 40 ms override remains a local trial setting.
Native automatic prebuffering starts at one frame so an empty producer queue
does not advance the audio cursor beyond future speech.

Before asynchronous uncork, prime two writes (about 10 ms) or the shorter
available source/terminal. Feeding continues while uncork acknowledgement is
pending. Drain and cork idle streams; new work may cancel a pending drain.
The shared progressive reserve, including the letter-specific threshold in
[ADR 0003](0003-progressive-audio-and-markers.md), remains independent of these
native priming requests.

Stream-wide stop discards local PCM, corks and flushes that lane, then admits its
new generation. It favors immediate retirement over the normal stream stop fade.
Selective keyed cancellation retains the common source fade without flushing
unrelated requests. PCM already consumed by PulseAudio/RDP cannot be recalled.
Completion and markers still describe source consumption, not physical output.

Connection setup, writes, native operations and drains have deadlines. Failure
retires that lane's active and queued sources. After a 250 ms admission cooldown,
fresh audio may reopen the connection on its source worker. There is no idle
retry loop or replay; another failed attempt retires its own backlog and repeats
the cooldown. Stop/drain remain independent of setup and shutdown permanently
closes admission. Initial connection failure fails startup. Output errors never
quarantine a healthy synthesis engine or trigger cross-engine replay.

## Consequences and alternatives

This bypasses ALSA while preserving the existing synthesis and timeline paths,
at the cost of native connection/thread ownership and lifecycle testing. The
default backend remains usable without libpulse. Replacing the entire audio
dependency stack is a separate decision; a native backend cannot repair a stalled
WSLg/RDP server. A Windows PCM bridge remains a possible later experiment.

Preserve blocked-producer, idle, replacement, cancellation-domain, connection
failure and shutdown tests. The [trial report](../experiments/2026-09-07-native-pulseaudio.md)
and [benchmark index](../benchmarks/README.md) record evidence and limits.
Smaller requested buffers alone establish neither acoustic latency nor parity.
