# ADR 0018: Duration-based initial playback reserve for letter navigation

- Status: Accepted under the maintainer's implementation request on 2026-09-20
- Refines: the initial reserve in ADRs 0006 and 0009 for legacy `l` requests

## Context

[DECtalk letter measurements](../benchmarks/2026-09-20-dectalk-letter-navigation.md)
found that first PCM was ready in about 2 ms, while the real-device three-window
reserve delayed short letters until native completion. Chunk counts have no
fixed duration across engines, conversion windows, trimming and speech rates.
Null output bypasses the reserve and cannot measure this delay.

## Decision

The server explicitly marks isolated `l` requests as letter navigation. Their
progressive playback source may attach after 40 ms of canonical, rendered PCM
has been supplied. Retain the existing three-window threshold as an alternative
release condition, so tiny windows cannot fill the fixed-capacity channel while
waiting for a duration target. Completion still releases a shorter source.
This is an audio reserve, not a timer or a 40 ms sleep.

Select the policy before publishing PCM. Ordinary queued speech, immediate
speech, timelines and previews retain their existing reserve. Buffered engines
and null output retain their existing behavior. No text-length heuristic,
runtime control, public wire field, dependency or helper change is introduced.
The same source policy covers the device and native PulseAudio backends;
PulseAudio's separate native priming and latency requests remain unchanged.

Retain all queue bounds, backpressure, cue ordering, cancellation, stop fades,
generation checks, engine isolation and the PCM commitment boundary. Preserve
character-rate scaling and the uppercase pitch cue. Do not change DECtalk's
native synchronization, cleanup, runtime qualification or parameter handling.

Record correlated first and final consumed-frame diagnostics for progressive
legacy letters. These are mixer-source timings, not physical acoustic onset.
Neither admission nor queued PCM establishes that playback started.

## Validation and limits

Test the frame threshold across different window sizes, early completion,
tiny windows, ordinary speech, cancellation before priming, stalled producers,
stop/recovery and marker order. Run matched muted device tests of actual `l`
requests, rapid replacement, and a range of letters and rates. Preserve raw
observations and separate platform tests from acoustic listening acceptance.

A smaller reserve can expose a slow producer sooner. It does not promise
gap-free output from every engine, device or scheduler. A stream shorter than
the target may still await completion; faster navigation may still wait for
native cleanup. Adjusting the chosen reserve requires retained device evidence.
