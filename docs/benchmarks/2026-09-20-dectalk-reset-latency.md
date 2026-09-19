# DECtalk startup: reset placement dominates ordinary speech

DECtalk's remaining ordinary-speech startup delay is largely time spent inside
`TextToSpeechReset(handle, false)`, called by Omnivox before every utterance.
Generating the first audio after text submission is much faster. The practical
optimization belongs in Omnivox's helper: change when it resets DECtalk, while
preserving cancellation, marker origins and voice-state isolation. These
measurements do not establish why the DLL's reset implementation takes so long.

This is an investigation with private experimental helpers, not a deployed fix.
The development launcher and running Emacs sessions were not changed.

## Where the time goes

Ninety warm line utterances, three independent workers with three warmups each,
using Perfect Paul at rate 225. Times below are milliseconds measured inside the
instrumented helper. The first audio buffer contained speech in every measured
utterance: its minimum peak was 3946 in signed 16-bit PCM.

| Phase | Median | p95 |
|---|---:|---:|
| Native reset before synthesis | 17.212 | 27.446 |
| Set rate | 0.001 | 0.001 |
| Build text with word indexes | 0.026 | 0.050 |
| Submit text through Speak | 0.064 | 0.359 |
| Text submission to first native PCM callback | 0.684 | 1.578 |
| First callback to first audio write | 0.145 | 0.230 |
| Encode/write first audio message | 0.024 | 0.045 |

The first callback-to-write interval includes Omnivox's intentional one-buffer
hold for late-arriving markers. A 512-sample buffer represents 46.4 ms of audio
at 11025 Hz, but DECtalk generates it faster than real time: holding one buffer
does not add 46.4 ms of wall-clock startup delay. Retaining that buffer remains
necessary for the current marker-before-audio guarantee.

Within the main server, queueing the first audio to first mixer-source
consumption took a median 0.044 ms, p95 0.111 ms. The native `TextToSpeechSync`
call took a median 19.989 ms, but callbacks delivered audio while it waited;
that whole duration must not be added to first-audio latency. Phases overlap,
and these medians are not an additive budget.

The instrumentation control measured stock/instrumented median startup of
20.126/20.615 ms and p95 of 30.996/32.734 ms. This is not a zero-overhead claim,
but the small difference cannot explain the 17 ms measured reset call.

For ordinary speech, the helper sends synthesis-started before entering native
synthesis, and the main server initializes its converter on that message. The
reset timing is consistent with the earlier result: removing roughly 10–16 ms
of converter setup did not improve DECtalk's median because that setup could
overlap its longer native reset. The earlier report's broad phrase "native
synthesis latency" should therefore be read more specifically as native reset
latency on this path.

## Controlled experiments

Simply omitting reset was deliberately tested as a diagnostic control. The
second utterance failed with `progressive native marker exceeds the completed
PCM`. The reset cannot just be deleted: subsequent native marker positions no
longer match the helper's per-utterance PCM origin. This failed control is
retained, not counted as a successful latency benchmark.

A second prototype omitted the initial reset and reset during the existing
cleanup block, before returning the synthesis result. It kept the same buffers,
marker holdback, wire format, cancellation reset and native voice restoration.
Ordinary speech still pays one reset per completed utterance, after streaming
its audio. Native edited speech already reset during cleanup; the prototype
also eliminates its separate preparation reset.

Each cell contains 90 warm observations in three shuffled blocks. All workers
used the same current main executable; only the selected helper changed.

| Workload / measure | Stock median | Prototype median | Stock p95 | Prototype p95 |
|---|---:|---:|---:|---:|
| Ordinary speech, first source | 20.422 | 2.962 | 32.681 | 3.782 |
| Ordinary speech, completion | 40.981 | 40.855 | 56.027 | 53.648 |
| Native `sm=61`, first source | 98.407 | 71.375 | 117.984 | 92.705 |
| Native `sm=61`, completion | 162.524 | 137.651 | 186.577 | 169.337 |

Ordinary startup improved by about 17.5 ms (85%), while total completion time
was essentially unchanged. This is moving necessary work away from first audio,
not making the DLL's reset itself faster. A request that arrives while cleanup
is running may still have to wait; these line comparisons wait for each prior
utterance to complete.

Edited native choices have an additional bottleneck. After moving reset,
preparation between setting the rate and building indexed text took a median
67.414 ms, p95 88.803 ms. That block applies and verifies the preset, common
mapping and edited parameters through three command/synchronization batches
and readbacks. The probes measure this block together, not the individual
command calls. Optimizing it is a separate investigation; these numbers do
not justify removing synchronization or readback checks.

## Checks and limits

The reset-after prototype passed:

- All 121 cases in the maintained DECtalk helper-6 acceptance suite: controls,
  voice defaults/restoration, readback, stale identities, cancellation and
  protocol-5 compatibility.
- A 100-synthesis streaming stress run with 11 cancellations, 6005 markers,
  varying common settings, and follow-up speech. The suite checks marker
  ordering, ranges and arrival before corresponding PCM.
- Forty measured server cases covering characters, words, lines, dense anchors,
  multipart presentation, rapid replacement, and simultaneous independent
  main/notification workers, plus warmups.

No production source was changed. The private prototype retains an unused
`synchronized` local, producing compiler warning CS0219; it is intentionally
kept in the recorded patch rather than polishing an experimental build after
measurement. Compilation used the maintained helper build script, pinned
Roslyn 5.6.0 and the same .NET 4 reference assemblies as the staged runtime.

These are Windows/WSL software measurements with null audio, not microphone
measurements or evidence of acoustic onset. Emacs Lisp is outside the timed
path. Only the installed DECtalk runtime and dictionary identified by hashes
were exercised. There is no macOS performance claim.

## Recommended implementation work

Use reset-after-synthesis as the starting point for a small helper change.
Explicitly cover initial readiness, failures during setup/reset/restoration,
and cancellation arriving during cleanup. Preserve the existing locks and
marker holdback, and verify that a failed reset cannot leave a reusable dirty
instance. Add focused lifecycle regression coverage before treating the
prototype's successful runs as a production guarantee.

Then use full Windows development staging because this changes a helper, and
verify actual navigation in a fresh development Emacs with audio enabled.
Benchmark native parameter preparation separately before changing its batching.
There is no current evidence that modifying DECtalk's own implementation is
necessary to obtain the measured ordinary-speech improvement.

## Evidence and repetition

[Index and provenance](data/2026-09-20-dectalk-reset/index.json),
[summary](data/2026-09-20-dectalk-reset/summary.json),
[correlated observations](data/2026-09-20-dectalk-reset/observations.json), and
[checksums](data/2026-09-20-dectalk-reset/SHA256SUMS) are retained with raw reports,
compressed logs, private source patches and the exact local invocation scripts.
Gzip entries, including the source patches, decompress to the original hashes
in the index. No executables or proprietary runtime files are included.

Helper phase timestamps use one `Stopwatch` clock; main lifecycle intervals use
the main process clock; client observations use `perf_counter_ns`. Requests are
correlated through job, dispatch and helper request IDs. No wall-clock timestamp
from one process is subtracted from another. Probe summaries are emitted after
synthesis, so diagnostic log formatting is outside first-audio timing.

The source basis is Omnivox `0b73df8`; the main binary is staged development
`f73e3fc2b777f977`, whose complete build provenance is retained. Decompress the
three `.patch.gz` files first. Reproduce by applying `probe.patch` to a private
copy of that source's `windows-helpers`,
building with the pinned helper toolchain, then applying `reset-after.patch`
to a separate instrumented copy and building again. Use explicit private
`OMNIVOX_DECTALK_HELPER` overrides; do not replace a live runtime's helper.
The retained runners read the local Emacsvox `.benchmarks/voices.json`; match
it to retained `plan.json`, including the main executable and external runtime
hashes, before repeating. Use new output directories.

To recompute statistics from this retained evidence:

```sh
python3 docs/benchmarks/data/2026-09-20-dectalk-reset/summarize.py
```

That command rewrites only the two derived JSON files beside the script. It
supports both the original plain logs and the retained compressed logs.
