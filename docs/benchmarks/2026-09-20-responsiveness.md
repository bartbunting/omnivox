# DECtalk completion and shared native-call handoff

Two measured optimizations are implemented and selected by the development
shortcut in runtime `577414114415d7b0`:

- `f8abb0b` requests finer Windows timer scheduling only while DECtalk owns a
  synthesis, through native reset and parameter restoration.
- `1dfe948` wakes admission when a native call releases its engine/process slot.
  `bd95655` releases that admission lock before cancellation or timeout reporting.

The existing 40 ms letter reserve, PCM conversion, markers, native calls,
cancellation deadlines and concurrency limits are retained. A separate private
cancellation experiment was not adopted.

## Letter playback

Matched, muted Windows device runs of actual `l` commands used Perfect Paul,
rate 85 and character scale 1.1. Two blocks reverse target order; each has four
measured requests after three warmups for every lowercase letter and uppercase
`A`. Requests are 300 ms apart. These are admission-to-mixer-source timings,
not physical acoustic onset.

| Letter | Before median, ms | After median, ms |
|---|---:|---:|
| a | 49.70 | 27.33 |
| e | 54.98 | 32.37 |
| o | 51.42 | 29.89 |
| v | 52.71 | 32.16 |
| z | 49.88 | 34.78 |
| A | 51.09 | 33.92 |

Together these previously slow letters moved from a median 50.77 to 31.63 ms.
Most other letters already started quickly: the equally weighted lowercase
alphabet median moved only from 10.96 to 10.16 ms. Its p95 improved from 59.23
to 32.59 ms. Across all 216 measured letter requests per target, synthesis-worker
time fell from a median 48.96 to 26.27 ms.

All 378 requests per target, including warmups, consumed their final frame.
The baseline reported eight device PCM waits; the candidate reported none.
This is evidence for these runs, not a guarantee of gap-free acoustic output.
Candidate rate-50 and rate-150 checks also passed.

## Where the delay came from

The local DECtalk source at `69ebb45` uses short polling sleeps in native
synchronization and reset. A private helper toggled only an active-synthesis
timer request, retaining the same native calls and checks. Four ordered blocks
(off, on, on, off), five texts, and 20 observations after three warmups produced
400 measured requests. Corresponding complete PCM hashes and marker sequences
matched exactly across all four blocks.

Client-observed helper completion typically fell from about 40–48 ms to 24–25 ms.
For `a`, median first PCM was 1.58/2.04 ms, last PCM 20.14/12.90 ms, and terminal
completion 40.60/24.60 ms. Thus the gain is principally completion, not faster
generation of the first samples.

The production helper balances each successful `timeBeginPeriod(1)` with
`timeEndPeriod(1)`, including exceptional exits. Unsupported resolution requests
do not prevent speech. Idle helpers retain no request. Windows determines the
actual scheduling; finer resolution can increase power use while active. See
[Microsoft's timer-resolution contract](https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod).

The shared converter is also relevant. Feeding its unchanged implementation
512 then 482 native frames at 11,025 Hz yields 1,016 canonical frames before
completion; `finish()` releases another 2,960, for the exact 3,976-frame total.
The helper terminal still follows native cleanup. Publishing a separate audio-end
boundary could release this tail sooner, but requires a negotiated protocol and
failure contract. The current stream cannot safely infer completion from a pause
or an incomplete callback. This pass adds no such protocol change.

## Shared admission and other speech paths

The old isolated-engine admission loop slept for 10 ms when a native owner was
still present. The new condition variable wakes it as soon as the last owner
releases capacity. Checking, publishing release and entering the wait share a
lock, preventing a lost wakeup. Cancellation still has its existing polling
bound; wakeup cannot bypass ownership or the two-call process limit.

A diagnostic extracted the actual before/after admission and lease code, replacing
the engine with a slot released after 2 ms. Three alternating blocks of 100 warm
observations per target on Linux measured release-to-acquisition median
8.023/0.037 ms and p95 8.081/0.072 ms. This isolates the polling cost; it is not
a claim that every speech request saves 8 ms.

Ordinary line comparisons used null output and three shuffled blocks of 30 warm
observations per engine and target:

| Engine | Before first source, ms | After first source, ms | Before terminal, ms | After terminal, ms |
|---|---:|---:|---:|---:|
| DECtalk | 2.89 | 3.72 | 40.03 | 26.67 |
| Eloquence | 5.39 | 5.55 | 8.84 | 9.18 |
| eSpeak | 3.03 | 3.27 | 32.30 | 32.71 |

A separate ordinary/custom DECtalk comparison used 40 observations per cell.
Custom first source moved from 3.00 to 3.75 ms, while completion improved from
67.56 to 37.52 ms. Ordinary first source in that comparison was 3.23/3.39 ms.
The measured 0.2–0.8 ms first-source increase is retained explicitly; this is
not a universal startup improvement. Eloquence and eSpeak receive no claimed
steady-speech speedup.

## Rapid replacement and further investigation

Three runs per target sent 60 letters at each of 100, 50, 30, 20 and 10 ms
intervals, followed by ten hard-stop/recovery pairs. Every burst's final request
started, all 30 recoveries per target passed, and routing stayed on Paul.
The candidate reported no PCM waits; the baseline reported one across these
burst runs. At 50 ms spacing the candidate started all 60 requests in each run,
versus 36–40 before. Started counts at shorter intervals include normal
coalescing and are not synthesis-throughput measurements.

At 10 ms spacing, final-request startup remained variable: baseline
44.73/117.55/44.47 ms; candidate 81.95/77.34/50.51 ms. A correlated trace shows
the candidate waiting for a cancelled helper request before submitting the
final letter. DECtalk's native reset-overlap path includes a 30 ms polling sleep;
finer timer resolution does not shorten a requested 30 ms sleep.

A private five-millisecond natural-completion grace experiment retained immediate
output suppression and deferred the forced native reset briefly. Two off/on
comparisons did not establish a consistent benefit across the repeat rates:
for the 10 ms bursts, final startup was 48.45/83.12 ms without grace and
55.96/42.51 ms with it. It also introduces an intentional native-stop delay.
The experiment is retained for investigation and is **not deployed**.

Further work should distinguish an audio-end notification from engine readiness,
and qualify native cancellation changes with controlled phase barriers before
changing their policy. The inspected shared reader's replaceable-input debounce
and cancellation polling are deliberate scheduling policies; neither was reduced
without corresponding semantic and native evidence.

## Verification and development deployment

- Locked workspace checks passed: 1,023 passing test results, including one
  subprocess rerun, and two explicit opt-in tests ignored. The lock-scope followup
  passed all 16 affected isolation tests and all-target CLI Clippy again.
- New tests cover notification after the last native owner, quarantine accounting,
  cancellation while waiting, and balanced/unsupported timer ownership. Existing
  native cleanup failures, restoration, readback and Stop overlap checks pass.
- DECtalk's 121-case helper-6 suite and a 200-utterance streaming stress run pass,
  including 21 cancellations, 12,002 markers and successful followup speech.
- Full Windows development staging, helper determinism and runtime verification
  pass. The final main-only followup reused that verified full package. The first
  full staging attempt collided with a running test helper; the successful retry
  serialized compilation and consumers.
- Fresh compiled Emacs acceptance passes both speech lanes, previews, native
  readback, palette save/reload and reconnection. Emacs byte-code and documentation
  preflights pass. No Lisp source changed.

The development launcher selects the verified candidate and the repeat benchmark
plan's current target was updated. Existing Emacs sessions retain their running
workers. Native macOS performance and acoustic listening were not measured here.

## Evidence and repetition

[Index and hashes](data/2026-09-20-responsiveness/index.json),
[summary](data/2026-09-20-responsiveness/summary.json),
[deployment](data/2026-09-20-responsiveness/deployment.json) and
[checksums](data/2026-09-20-responsiveness/SHA256SUMS) retain raw observations,
compressed logs, both experimental patches, exact runtime provenance and runners.
No executable or proprietary runtime is included.

Baseline is `6414dbbfa3e8543a`; candidate is `577414114415d7b0`, built from
`bd95655`. Both use Rust 1.97.1 and the same Windows GNU target/features, installed
DECtalk DLL/dictionary and eSpeak data. The production helper changed; its hash is
bound separately. All compilation completed before matched measurements.

Recompute retained summaries with:

```sh
python3 docs/benchmarks/data/2026-09-20-responsiveness/summarize.py
```

For a new run, use the recorded commands and plan in a new output directory after
checking all local paths and runtime hashes. Letter and burst runners are retained
with the preceding [letter-reserve evidence](data/2026-09-20-letter-playback-reserve/index.json).
The existing Emacsvox `voice_benchmark.py` remains the maintained ordinary/custom
speech harness. Do not compare compiler, helper or configuration changes as if
they were otherwise identical source builds.
