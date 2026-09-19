# Progressive PCM resampler reuse

The initial release-versus-development comparison mixed MSVC and GNU Windows
binaries. It established a latency difference inside Omnivox, but did not establish
that native voice parameters introduced a source-code regression. A historical
GNU build from before that work shows similar latency to the current GNU build.

Direct measurement of the optimized Windows audio library found repeated sinc
filter construction taking about 10–16 ms, with some higher outliers. Temporary
CLI probes put first-audio forwarding/playback setup below 1 ms. Those probes
were removed before the measured optimization build.

The change exclusively checks out an existing filter of the exact sample rate
and channel count, and resets it before returning it to a four-entry idle pool.
No active filter is shared. Retained sinc tables total roughly 1 MiB per process;
the pool retains no voice models or utterance content. Filter coefficients,
interpolation, frame mapping and callback-boundary handling are unchanged.
A cache miss still pays construction cost. Direct constructor measurements after
warmup were 0–1 microseconds at the measurement's integer-microsecond resolution.

## Results

All 36 jobs completed, retaining 4,548 timing samples. Each warm line cell below
contains 90 observations. Times are milliseconds from dispatch to first source.

| Engine | Before median | After median | Before p95 | After p95 |
|---|---:|---:|---:|---:|
| eloquence | 16.36 | 4.97 | 20.00 | 5.89 |
| espeak | 15.41 | 2.81 | 19.14 | 3.63 |
| dectalk | 20.25 | 20.56 | 33.25 | 34.29 |

No warm series crossed the comparison screen of a p95 increase greater than
both 5 ms and 15%. Three warm server-readiness series had only three samples and
were marked insufficient. This is a regression screen, not statistical proof.
DECtalk remains dominated by native synthesis latency.

Historical control medians (90 warm lines per cell) were:

| Engine | Before native controls, GNU | Current before optimization, GNU |
|---|---:|---:|
| eloquence | 14.87 | 15.61 |
| espeak | 15.65 | 15.61 |
| dectalk | 20.35 | 20.17 |

That control does not reproduce the roughly 10 ms increase inferred from the
earlier MSVC-versus-GNU comparison. It does not rule out smaller changes or
regressions in other paths.

## Method and provenance

Windows x64 under WSL; null audio; exact Eloquence v1, DECtalk paul and eSpeak
espeak:gmw\en; rate 225. Client observations use Python perf_counter_ns. The
measured event is first mixer-source consumption, not physical acoustic onset.
Emacs and Emacsvox Lisp are absent from these timed server jobs. The Emacsvox
voice_benchmark.py orchestrator uses the maintained Omnivox benchmark backend;
its hashes, executable/helper hashes and external-runtime hashes are retained.

Three shuffled blocks, 30 measured warm requests after three warmups per case
and target, and three cold requests per case per block. Both targets run in the
same benchmark invocation, with compilation finished first. Workloads include
characters, words, lines, dense anchors, multipart timelines, replacement and
simultaneous independent main/notification workers. The known eSpeak dense-anchor
failure is explicitly excluded, as in the earlier run; it is not fixed here.
Cold series have only nine observations and cannot support tail-latency claims.

Before is development 6e55a493cb9d2562; after is f73e3fc2b777f977. Both use Rust
1.97.1, x86_64-pc-windows-gnu, the recorded optimized release flags, and no Piper
feature. The after package was built from eefa911 with the audio source change
now committed as e0efb06; subsequent changes are documentation only. External
Eloquence/DECtalk runtimes and eSpeak data are identical across the comparison.
No tagged release was created.

The separate historical compiler control uses e1ecdb481ee08fd0 (clean f10962b951,
before native controls) versus 6e55a493cb9d2562, with 90 warm line observations
per engine and target. Both use the same Rust/GNU compiler settings. The older
package includes Piper while current does not, so this is a compiler-matched
historical control, not an otherwise identical build configuration.

## Verification notes

- The fresh locked workspace passed 1,014 distinct tests plus one child rerun;
  two tests were intentionally skipped. Seven PCM tests include exact sample
  equality after completion/cancellation, exact format matching, exclusive
  checkout, a fixed idle limit, scaled frame counts and sinc quality.
- The full workspace rerun used --test-threads=1. Two unrelated catalogue timing
  tests failed during a heavily loaded parallel run and passed on that rerun.
- Workspace Clippy with all targets, Windows GNU audio Clippy with all targets,
  formatting and whitespace checks passed.
- Full Windows development staging and runtime verification passed. The
  main-only guard had rejected the initial older staged base, so no reuse guard
  was bypassed.
- Fresh compiled Emacs native-control acceptance passed against the new runtime:
  DECtalk/Eloquence editor previews, comparisons and readback; temporary palette
  save/reload; both speech streams; and main-worker reconnection.
- Validation used a fresh, short on-disk Cargo target directory. An old shared
  cache pointed to another checkout and omitted the new tests; those results
  were discarded. A longer fresh directory exceeded upstream Piper/eSpeak path
  limits, and /tmp ran out of space; final checks used /home/bart/ovx-lat20 and
  the existing verified Piper inputs.

No physical onset or macOS performance measurement is claimed. Existing Emacs
sessions stay on their running binaries until restarted.

The development launcher now selects the verified after package. Its previous
contents are retained locally for rollback. The local repeat plan uses
6e55a493cb9d2562 as its baseline and f73e3fc2b777f977 as current, so subsequent
default comparisons use the same Windows compiler. The original release
comparison and this run remain unchanged.

## Retained evidence and repetition

[Benchmark index](data/2026-09-20-progressive-resampler/index.json) retains
the exact configuration, hashes, summary and paths to all unmodified reports.
[Comparison](data/2026-09-20-progressive-resampler/comparison.md),
[historical control](data/2026-09-20-progressive-resampler/compiler-control/manifest.json),
[compiler evidence](data/2026-09-20-progressive-resampler/compiler-evidence.json) and
[SHA256SUMS](data/2026-09-20-progressive-resampler/SHA256SUMS) are included.
Constructor probes are diagnostic samples, not the end-to-end measurements.

From the sibling Emacsvox checkout, repeat with the retained plan after checking
its local executable and runtime paths:

```sh
python3 utils/voice_benchmark.py run \
  ../omnivox/docs/benchmarks/data/2026-09-20-progressive-resampler/plan.json \
  .benchmarks/resampler-repeat --preset full
```

Use a new output directory each time. The development launcher's
`--benchmark-voices` option calls the same harness. For source-change attribution,
match compiler, target, features, build profile, helper and runtime inputs;
record and isolate any differences before assigning the cost to a source change.
