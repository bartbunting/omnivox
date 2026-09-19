# DECtalk reset deployment and custom-parameter timing

Follow-up: [batched custom parameters](2026-09-20-dectalk-batched-parameters.md)
remove these pre-speech waits for warm plain-text requests while retaining
actual readback before output. The measurements below describe the earlier build.

The reset-placement change is implemented in `5b7f1b7` and staged in development
runtime `34e353095d7cfd73`. The desktop development launcher selects that package.
Custom-parameter preparation remains expensive because the helper calls
`TextToSpeechSync` after three separate command batches. Almost all of the
measured preparation time is in those waits, rather than parameter composition,
command submission or readback.

## Implemented reset change

The helper resets after each utterance's audio has been streamed. Cleanup keeps
the existing cancellation lock, drains native work after failures, and restores
and verifies native voice state when required. Capture setup now sits inside
the cleanup scope. A failed reset or restoration makes the instance unusable
until helper restart; subsequent requests cannot submit more native work.

The new lifecycle audit fails against the previous helper with "reset delayed
first audio" and passes against the new binary. It exercises first and repeated
speech, setup failure and recovery, reset/restoration failure followed by refused
reuse, and Stop blocked during cleanup without performing a second reset.

The complete runtime audit passed 1,135 captures across all nine voices, 45
cancellations, 18 delivery failures, 27 progressive cases and nine reset/restore
overlaps. All 121 helper-6 wire acceptance cases also passed. The source-contract
suite, documentation checks and whitespace checks passed.

Full `make windows-omnivox-dev` staging passed deterministic helper compilation,
payload checksums, engine inventories and companion synthesis checks. The
packaged helper has the same hash as the tested helper. Fresh compiled Emacs
acceptance passed DECtalk/Eloquence editor preview/comparison/readback, palette
save/reload, ordinary speech on both streams, and main-worker reconnection.
Those tests use null audio and isolated settings.

The development launcher was repinned after staging, and its local repeat plan
now compares the previous development package with the new one. The pre-existing
default runtime symlink was restored after staging so other profiles keep their
previous default. Existing Emacs processes were not restarted.

## Server measurements

Ninety observations per cell, in three shuffled blocks with three warmups per
worker. Before is `f73e3fc2b777f977`; after is `34e353095d7cfd73`. Both use Rust
1.97.1, Windows GNU release builds, the same installed runtime/dictionary, exact
Perfect Paul and rate 225. Compilation and integration checks finished before
these sequential timing runs.

| Workload / measure | Before median | After median | Before p95 | After p95 |
|---|---:|---:|---:|---:|
| Ordinary first source | 27.166 | 2.854 | 34.397 | 3.650 |
| Ordinary completion | 50.464 | 50.120 | 63.666 | 59.480 |
| Native `sm=61` first source | 105.931 | 81.163 | 123.091 | 96.225 |
| Native `sm=61` completion | 182.086 | 156.147 | 199.904 | 176.376 |

All values are milliseconds from dispatch. This run's baseline is higher than
the earlier investigation's baseline; comparisons above use the paired runs,
not observations from different sessions. Ordinary completion remains similar
because reset was moved rather than eliminated. Native edited speech previously
reset both before preparation and during cleanup, so it also avoids one reset.
These are software first-source measurements, not physical acoustic onset.

## What custom parameters cost

A separate private x86 STA process loads the exact compiled helper and wraps
four native delegates: Speak, Sync, Reset and GetSpeakerParams. It records
`Stopwatch` durations in memory, restoring the delegates before disposal. It
uses buffered silent capture, three warmups and 90 measured requests per case,
shuffled in three blocks. All requested fields must match the application
readback, and each request must produce valid PCM. No hooks enter a deployed
helper or running Emacs session.

| Preparation before speech | Sync calls | Median time in Sync | Median other preparation |
|---|---:|---:|---:|
| Ordinary path | 0 | 0 ms | Not measured by this probe |
| Native path, empty edit map | 2 | 50.744 ms | 0.295 ms |
| Native path, `sm=61` | 3 | 77.947 ms | 0.500 ms |
| Native path, all 28 fields | 3 | 76.697 ms | 0.499 ms |

The empty native map is a diagnostic request explicitly using the native path;
it does not mean ordinary unedited voices always pay this cost. Normal ordinary
requests submit their common prefix together with the spoken text.

For one edited parameter, the three Sync calls had median durations of:

1. Select pristine preset, then wait: **26.469 ms**.
2. Apply common controls, then wait: **26.367 ms**.
3. Apply native edits, then wait: **26.765 ms**.

The three command submissions together took a median **0.217 ms**, and their
three native readbacks together took **0.081 ms**. The remaining preparation
figure above also includes managed composition, validation and receipt callback
overhead. More than 99% of the measured preparation time is in synchronization.
Individual medians need not sum to the median of a per-request total.

These delegate measurements include reflection overhead and have a different
host/capture path from the server benchmark. They establish where time is spent;
they must not be presented as independent end-to-end startup measurements.

## Why synchronization is expensive

`TextToSpeechSync` waits for the engine's queues to finish. The inspected source
calls its text-queue wait twice and then its pipe-empty wait. The installed DLL
also contains these calls and polling sleeps: the Sync export resolves to
`0x1c007720`, calls the queue wait at `0x1c008330`, and that routine calls the
imported `Sleep` function with argument 5 at `0x1c00838e`. The pipe wait also
contains sleep-based polling. This corroborates the measured waiting cost;
the investigation does not establish the exact contribution of each sleep or
Windows scheduling to each observed interval.

The local DECtalk source inspected was commit
`69ebb459137a7a8d92ed41da8362233eaa173efc`. Its corresponding source is useful
context, but exact source-to-installed-DLL build provenance is not claimed.
The retained disassembly identifies the actual hashed installed binary.

## Next optimization

The useful next experiment is to compose the complete desired state and apply
it as one batch, retaining final readback before speech. Intermediate waits are
currently used to obtain the pristine preset and the runtime's common mapping.
Replacing them needs qualified preset/common-state planning or a carefully
scoped cache, with checks for voice changes, defaults, contextual precedence,
native command effects, cancellation and failed application. It should preserve
the existing common mapping and avoid carrying edits between utterances.

One remaining Sync may still cost tens of milliseconds. Matching ordinary
startup more closely would need a further measured strategy for confirming
application, such as a qualified native API or completion signal; simply
deleting waits can read stale state. No custom-parameter batching, cache,
native API binding or wire contract was changed in this deployment.

## Evidence and repetition

[Index and provenance](data/2026-09-20-dectalk-parameter-timing/index.json),
[server summary](data/2026-09-20-dectalk-parameter-timing/server-summary.json),
[parameter summary](data/2026-09-20-dectalk-parameter-timing/parameter-summary.json),
and [checksums](data/2026-09-20-dectalk-parameter-timing/SHA256SUMS) accompany
unmodified raw results, compressed logs, the timing probe and invocation scripts.
Gzip files decompress to the original hashes in the index. The implementation
commit also retains the [runtime/lifecycle audit](data/2026-09-20-dectalk-reset-execution.json)
with line endings normalized to LF; its original bytes are in the new bundle.

To repeat the native-call audit, run the retained `time-parameters.ps1` in x86
Windows PowerShell with `-Sta -Helper <verified helper> -RuntimeDll <installed DLL>`.
Keep its `DectalkTimingAudit.cs` alongside it. Redirect stdout to a new
`parameter-timings.json` and stderr to a separate log. Run
`summarize-parameters.py <results-directory>` to produce the phase summary.
The probe captures silent PCM and never installs or modifies runtime files.
