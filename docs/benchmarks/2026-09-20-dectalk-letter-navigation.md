# DECtalk letter navigation: synthesis and playback buffering

Investigation against development runtime `b98c08411e68a656`, containing
Omnivox `8119e02`, with the same qualified Windows x86 DECtalk DLL as the
[custom-control measurements](2026-09-20-dectalk-batched-parameters.md).
No speech implementation, runtime selection, or live Emacs state was changed.

## Finding

The legacy `l` command does not take a slower DECtalk synthesis path. It queues
audio in about 2 milliseconds when the worker is idle. However, real-device
playback has a three-window reserve that null playback deliberately bypasses.
Short sounds can wait for synthesis completion before this reserve is released.
Matched one-character requests through the same progressive pipeline reach the
device mixer about 40 milliseconds later than a longer letter name does.

This identifies a playback-buffer optimization opportunity inside Omnivox.
It does not establish an improvement yet, and it is not a measurement of
physical sound at the listener's ears.

## Path inspection

Alphabetic character navigation in Emacsvox calls `tts-letter`, which sends
`l {character}`. The Omnivox reader stops earlier speech playback, invalidates
obsolete requests and queues a `Letter` request. It does not synchronously stop
every engine. The worker's `process_letter` applies the character-rate scale,
lowercases the text and supplies the established uppercase pitch cue. It then
uses the same routed progressive synthesis function as ordinary speech.

The letter route selects the legacy physical voice; it does not perform the
native-control editor's parameter checks. DECtalk submits a forced utterance,
streams its PCM, then synchronizes and resets its native state. Those last
operations keep the synthesis worker occupied for about 40 milliseconds total,
even though the first PCM is already available.

`omnivox-audio/src/output.rs` attaches a device source after three nonempty
audio windows, or when a shorter stream completes. Null sources attach
immediately. This distinction is intentional and recorded in
[ADR 0006](../adr/0006-bounded-progressive-synthesis.md). A queued-audio timestamp
therefore does not establish that device playback has started.

The older standalone Windows DECtalk bridge documents a dropped-character
problem with `TextToSpeechTyping` immediately after a normal stop. Substituting
that API is not an established solution to this playback-buffer delay.

## Measurements

All processes were private and ran serially. Voice: Perfect Paul. Protocol
speech rate: 85. No concurrent builds. Each isolated cell has 20 measured
requests after three warmups. Source and payload identities, exact probe
scripts, observations and diagnostic logs accompany this report.

### Exact legacy commands, null audio

Character scale was 1.0 to match the ordinary request's synthesis rate. These
are server admission-to-audio-queue measurements, not playback onset.

| Command | Median first audio queued, ms |
|---|---:|
| `l {a}` | 1.854 |
| `tts_say {a}` | 2.047 |
| `l {A}` | 2.183 |
| `l {b}` | 2.354 |
| `l {w}` | 2.453 |

Median worker occupancy was approximately 39–40 milliseconds for every row.

### Matched playback probes

The legacy `l` command has no public playback-start event. To measure the
buffering boundary without changing production code, the second probe sends
the same lowercase one-character text through tracked ordinary speech. Source
inspection confirms both enter the same progressive synthesis and playback
buffering functions. These results must not be labelled exact `l` key-to-sound
latency.

The device probe sets output voice volume to zero, after silence trimming, so
the real device path consumes the original frame counts without playing a
sample aloud. The null probe uses identical requests and settings. Times run
from client dispatch to receipt of the first mixer-source marker and include
transport and scheduling. They exclude Emacs key handling and acoustic latency.

| Text | Null median, ms | Device median, ms | Device p95, ms |
|---|---:|---:|---:|
| `a` | 2.678 | 49.761 | 59.433 |
| `b` | 2.719 | 49.460 | 59.847 |
| `w` | 2.528 | 9.486 | 9.936 |
| `latency` | 3.258 | 9.625 | 19.283 |

The short-source delay matches waiting for the worker's completion followed by
device scheduling. Longer sounds provide enough windows to begin much earlier.
The measurements and shared path make the reserve the primary explanation;
an actual buffering change still needs a matched before/after device test.

### Exact `l` requests during rapid replacement

Null-audio bursts cycle `a`, `b`, `w`, with character scale 1.1, matching the
development session's configured scale. Each burst has 63 submitted commands;
the first three are omitted from the summary. Intervals are client sleep
targets, not guaranteed server admission intervals.

| Interval | Requests with audio queued | Median queue latency, ms | p95, ms |
|---|---:|---:|---:|
| 100 ms | 60 / 60 | 2.655 | 3.487 |
| 50 ms | 60 / 60 | 2.412 | 3.206 |
| 30 ms | 35 / 60 | 14.133 | 29.846 |
| 20 ms | 24 / 60 | 10.300 | 18.251 |

At faster rates, newer requests supersede older ones while the worker completes
cleanup. The smaller 20 ms figure describes surviving requests, not better
throughput or successful speech of every character. There was no accumulating
multi-second queue in these bursts.

## Proposed follow-up

Evaluate a short initial reserve measured in audio duration for interrupting
letter navigation, instead of requiring three producer-dependent windows.
Keep the bounded queue, stop fade, marker ordering, cancellation and engine
isolation. Choose the duration from matched device tests; 40 milliseconds of
saved startup here is an opportunity, not a promised result.

That is a change to the accepted playback policy, so its design must reconcile
ADR 0006 and the PulseAudio decision before implementation. Validate short and
long letters, uppercase cues, fast replacement, deliberately stalled producers,
device underruns and stop/recovery on the supported output backends. Retain a
muted device benchmark alongside null-audio timing: null alone misses this
delay. Native cleanup is a separate secondary target for faster repeats.

## Evidence and repetition

[Provenance and invocations](data/2026-09-20-dectalk-letter-navigation/index.json)
identify all four runs. The retained probes accept a fresh output directory and
an explicit `--program` path. They use the maintained
`tools/benchmark_server.py` transport and local WSL launcher; their local paths
and runtime environment must be adapted elsewhere. No probe modifies a live
Emacs session or its settings. The device probe intentionally mutes its private
worker's speech output.

Raw observations, summaries and compressed stderr logs are retained with
[checksums](data/2026-09-20-dectalk-letter-navigation/SHA256SUMS). The original
isolated-letter probe labelled millisecond values with `_us` suffixes inside
`timings_ms`; the retained copy and data correct those suffixes to `_ms` without
changing values. The provenance records the original-byte hashes.
