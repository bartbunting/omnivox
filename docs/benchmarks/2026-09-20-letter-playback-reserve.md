# Letter playback with a duration-based initial reserve

Implemented in `3eec4f3`, with visible stall diagnostics in `49b2cfe`.
The development shortcut now selects runtime `6414dbbfa3e8543a`.

## Result

On this Windows DECtalk installation, 20 of the 26 lowercase letters had
substantial startup improvements. In the alphabet sample, their per-letter
medians moved from roughly 40–60 ms to 4–14 ms. The median over all 130 measured
alphabet requests fell from 49.931 ms to 10.619 ms. This is software mixer-source
startup, not physical acoustic latency or a natural-language weighted average.

Some short sounds still await DECtalk's final output. The early window for `a`
contained only 447 canonical frames, about 10 ms. A 20 ms reserve experiment
did not improve `a` or uppercase `A`, so the final setting retains the larger
40 ms reserve. Native synchronization, cleanup and final resampler output remain
separate optimization opportunities.

## Implementation

The legacy `l` handler marks only its private synthesis context as letter
navigation. After 40 ms of rendered PCM is supplied, the progressive producer
may attach to real playback. The existing three-window condition and completion
also release the reserve, preserving progress for tiny windows and short streams.
There is no added sleep. Ordinary speech, timelines, previews, buffered engines
and null output retain their previous buffering behavior.

PCM, trimming, resampling, character-rate scaling, uppercase pitch, marker order,
queue capacity, cancellation and native helper behavior are unchanged. See
[ADR 0018](../adr/0018-letter-navigation-playback-reserve.md).

Progressive `l` requests now log correlated first and final consumed frames.
Device playback reports the first PCM wait longer than its 2 ms receive poll,
once per waiting interval. Null output does not report these expected producer
waits. One early-window diagnostic per device letter records the available
frame count. These entries use the normal info-level logger; this CLI does not
enable debug logging through the probes' `RUST_LOG` environment setting.

## Matched measurements

Baseline: `f8b3b04d3243ee76`, the preceding behavior with first-frame diagnostics.
Final: `6414dbbfa3e8543a`. Both use the same qualified x86 DECtalk DLL, dictionary
and helper. Exact legacy `l` commands select Perfect Paul, rate 85 and character
scale 1.1. Each row has 20 measured requests after three warmups. Requests start
300 ms apart, in private processes with output volume zero after trimming.
Builds and acceptance completed before timing; benchmark processes ran serially.

| Letter | Before median, ms | After median, ms |
|---|---:|---:|
| `a` | 51.752 | 49.841 |
| `b` | 46.832 | 5.171 |
| `w` | 11.703 | 10.467 |
| `A` | 47.813 | 48.120 |

The separate alphabet comparison has five measured requests per lowercase
letter after three warmups. `a`, `e`, `o`, `v` and `z` still had medians near
50 ms; `w` was already fast. Equal alphabet weighting makes this a reproducible
comparison, not a claim about the exact improvement for a user's text.

## Playback and replacement limits

All 92 isolated DECtalk requests and 44 Eloquence requests, including warmups,
completed without a reported device PCM wait. The 208-request DECtalk alphabet
run reported four waits: measured `p`, two measured `x` requests, and one `r`
warmup. Native worker occupancy in those cases was about 61–69 ms. Every alphabet
request reached its final frame. The wait diagnostic establishes a software
producer stall; these muted tests cannot establish whether it was audible.
Gap-free acoustic output is not claimed.

The final rapid-navigation run sent 60 letters at each target interval of 100,
50, 30, 20 and 10 ms. It started 60, 43, 17, 7 and 3 requests respectively; newer
requests supersede older ones. The final request of each burst started, no
synthesis routed away from Paul, and all ten hard-stop/recovery sequences
succeeded. Two PCM waits were reported during this run. Fewer started requests
at extreme repeat rates are coalescing, not improved throughput. Native worker
cleanup remains a limit on rapid replacement.

Additional rate-50 and rate-150 probes passed with the same 40 ms policy before
the logging-only follow-up. Eloquence device results remained in the same short
scheduling range, with medians varying by several milliseconds in both
directions; this is not evidence of a specific Eloquence speed improvement.

## Validation and deployment

- Locked workspace tests: 1,021 passed, two explicit opt-in tests ignored
  (long eSpeak stress and separately configured native routing).
- Affected all-target Clippy, formatting, documentation and whitespace checks
  passed. New tests cover frame thresholds, exact PCM/cues, short completion,
  tiny windows, ordinary speech, cancellation, stalled producers and recovery.
- Full Windows development staging built the instrumented baseline. Guarded
  main-only staging then verified the candidate and reused unchanged helpers.
- Fresh compiled Emacs acceptance passed previews, native settings/readback,
  main and notification speech, and reconnection with the final launcher bytes.
  Emacs byte-code and documentation preflights passed.
- The development shortcut was updated; existing Emacs sessions and the default
  runtime pointer were preserved. Reopen development Emacs to load this build.

Real-device measurements cover Windows x64 GNU with this installed DECtalk
runtime. Linux tests exercise the shared reserve through the fake PulseAudio
device. macOS native playback and physical listening were not tested here.

## Evidence and repetition

[Provenance, build identities and commands](data/2026-09-20-letter-playback-reserve/index.json),
[deployment record](data/2026-09-20-letter-playback-reserve/deployment.json) and
[checksums](data/2026-09-20-letter-playback-reserve/SHA256SUMS) accompany raw
observations, compressed logs, the baseline instrumentation patch, gate results
and repeat scripts. The probes use the maintained `tools/benchmark_server.py`
transport and local WSL paths; adapt those paths for another installation.
They start private workers, mute device output and do not edit live Emacs state.
The 20 ms experiment is retained as investigation evidence and was not deployed.
