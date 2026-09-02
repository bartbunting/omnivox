# Windows x64 Eloquence and DECtalk Streaming Comparison, 2026-09-03

This records the matched Eloquence and DECtalk null-output rerun after their
native callbacks began feeding helper protocol v5 and the main server enabled
progressive playback with native marker delivery.

## Scope and provenance

- Target: `x86_64-pc-windows-gnu`, run through Windows interoperability from
  WSL2 on `Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.43`.
- Before: build `f7204ac69b6010f1`, Omnivox commit
  `2973d5efc86d40520f146bd635209e40b4154c9e`.
- After: build `09b89b3ff537d12b`, Omnivox commit
  `24c0fe35dbcffb9124bd215d46ee976567ce3cb5`.
- Emacsvox deployment commit for both:
  `3c47d0a2afa46ad4c9a32a5215bcddc4e7bdb019`.
- Server version for both: `1.6.4`.
- Exact engines and voices: Eloquence `v1` and DECtalk `paul`.
- Output: explicit null backend; no audio was played.

Both payloads use the supported development staging path. The after payload's
recorded Omnivox and Emacsvox tracked-worktree hashes are the SHA-256 empty-diff
hash, so its staged source matches the named commits.

## Method

The [streaming plan](plans/2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b.json)
matches the two relevant runs in the frozen
[pre-optimization plan](plans/2026-09-03-windows-x64-null-f7204ac69b6010f1.json):
20 recorded iterations for character, word, line, dense-action, multipart, and
five-dispatch replacement workloads in both cold and warm modes, with two
unrecorded warmups before each warm case. Each engine contributes 240 recorded
winner samples after the change.

Every after sample completed through the expected engine and exact physical
voice. All replacement winners observed the other four dispatches cancelled.
Measurements use `time.perf_counter_ns`; source is client receipt of the first
`utterance_started` event from the null source, not physical audio onset.
Terminal timing excludes waveform playback duration.

Character, word, line, multipart, and replacement requests carry no requested
anchors and use progressive playback. Their native word, sentence, phoneme, or
index markers are remapped after silence trimming and queued before playback
crosses the corresponding frame. Dense-action requests carry presentation
anchors and deliberately retain whole-result rendering.

## Results

Warm dispatch-to-source p50 for Eloquence, in milliseconds:

| Workload | Before | Streaming | Change |
| --- | ---: | ---: | ---: |
| Character | 15.473 | 2.738 | 82.3% faster |
| Word | 16.824 | 2.693 | 84.0% faster |
| Line | 31.497 | 4.081 | 87.0% faster |
| Dense | 43.327 | 50.386 | 16.3% slower |
| Multipart | 36.493 | 4.872 | 86.6% faster |
| Replacement | 80.274 | 33.878 | 57.8% faster |

Warm dispatch-to-source p50 for DECtalk, in milliseconds:

| Workload | Before | Streaming | Change |
| --- | ---: | ---: | ---: |
| Character | 71.997 | 31.881 | 55.7% faster |
| Word | 74.555 | 31.691 | 57.5% faster |
| Line | 81.581 | 31.273 | 61.7% faster |
| Dense | 90.325 | 71.717 | 20.6% faster |
| Multipart | 84.129 | 31.720 | 62.3% faster |
| Replacement | 119.664 | 57.416 | 52.0% faster |

The after warm source p95 values for the progressive character, word, line,
and multipart cases were 3.627, 3.592, 4.934, and 5.660 ms for Eloquence, and
32.777, 37.655, 32.948, and 39.371 ms for DECtalk. Replacement p95 was 41.307
ms for Eloquence and 64.410 ms for DECtalk. The anchored dense control's p95
was 53.181 ms for Eloquence and 104.052 ms for DECtalk.

Warm dispatch-to-terminal p50 also decreased on every DECtalk workload and on
five of six Eloquence workloads. The largest changes were Eloquence character
from 15.541 to 5.874 ms and DECtalk multipart from 85.955 to 61.993 ms. The
buffered Eloquence dense control changed from 69.600 to 73.314 ms. Streaming is
expected to affect source onset more strongly than terminal completion.

Across progressive cold cases, dispatch-to-source p50 improved by 30.5% to
56.0% for Eloquence and by 37.5% to 50.0% for DECtalk. Cold startup remained
the dominant cost: process-start-to-source medians ranged from 602.298 to
669.070 ms for Eloquence and 674.882 to 729.959 ms for DECtalk across those
cases.

## Interpretation and limitations

The ordinary warm results show that both native callback paths now reach the
main server's bounded playback source before whole-utterance synthesis ends.
DECtalk intentionally retains one 512-sample native block so a marker reported
a few samples late by the runtime can still be put on the wire before its
audio. This is 46.4 ms of 11.025 kHz speech data, not a 46.4 ms real-time wait;
native synthesis produces callbacks faster than playback.

A fixed-text WAV comparison found almost unchanged post-trim duration:
Eloquence changed from 135,024 to 135,023 canonical frames, while DECtalk
changed from 141,959 to 141,840 frames (2.70 ms). Protocol-v5 canonicalization
uses continuous linear interpolation across callback boundaries instead of the
old whole-result sinc converter, so these timing checks do not replace an
audible quality test.

- This was a later two-engine run rather than an interleaved paired trial;
  host drift can affect the comparison, especially the buffered dense case.
- Twenty samples per group characterize this development machine, not every
  Windows system.
- Null output excludes waveform duration, device buffering, underruns, and
  physical acoustic onset.
- Eloquence remains an external licensed installation. DECtalk used the pinned
  runtime recorded by the staged provenance.
- Requests containing presentation anchors still use the buffered path so
  timeline effects and semantic events retain exact whole-window rendering.

## Raw evidence

- [Suite index](data/2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b/suite.json)
- [Eloquence JSON](data/2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b/repeat-001-order-001-eloquence-v1.json)
- [DECtalk JSON](data/2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b/repeat-001-order-002-dectalk-paul.json)
- [SHA-256 manifest](data/2026-09-03-windows-x64-null-eloquence-dectalk-streaming-09b89b3ff537d12b/SHA256SUMS)
- [Before Eloquence JSON](data/2026-09-03-windows-x64-null-f7204ac69b6010f1/repeat-001-order-001-eloquence-v1.json)
- [Before DECtalk JSON](data/2026-09-03-windows-x64-null-f7204ac69b6010f1/repeat-001-order-005-dectalk-paul.json)
