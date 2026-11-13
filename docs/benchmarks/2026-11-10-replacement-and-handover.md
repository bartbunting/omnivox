# Prompt Flite replacement and foreground restart handover

Two bounded changes address the blockers found in the
[eight-engine Windows report](2026-11-10-windows-multi-engine.md): Flite now asks
for cancellation promptly on soft replacement, and Emacsvox prepares replacement
foreground voices while old speech remains available. Notification startup no
longer delays foreground configuration. The guard remains opt-in.

These are client-observed source playback measurements with null output. They
are not first-PCM measurements, acoustic onset or listening acceptance. No
transport hop, process boundary, ownership proof or managed activation guarantee
was removed. Accepted ADRs remain unchanged.

## Flite replacement

The shared native execution wrapper waited up to 75 ms after soft supersession
before asking an engine to stop. That protects engines where stopping is costly,
but Flite observes cancellation through its native streaming callbacks and can
retain the loaded voice. The qualified exception asks Flite to stop at the next
cancellation check, retaining the 10 ms polling interval, actual native ownership,
250 ms helper cancellation watchdog and bounded admission. Other engines retain
their existing behaviour. A cancelled task still occupies its slot until it exits.

The phase probe varies the delay after observing old source speech: 0, 10, 25,
50 and 100 ms. Actual phases include approximately 6 ms of client/reload work and
are retained separately. Each replacement overlaps a real preference reload and
notification speech. Minimal profiles enable eSpeak/Flite; full profiles enable
all eight qualified engines. Both ordinary and guarded paths use direct speech
pipes. Runs reverse path order in the second block.

Median replacement source latency in milliseconds:

| Requested phase | Minimal baseline ordinary / guarded | Minimal candidate ordinary / guarded | Full baseline ordinary / guarded | Full candidate ordinary / guarded |
| --- | --- | --- | --- | --- |
| 0 ms | 94.71 / 90.91 | 15.05 / 14.99 | 95.91 / 92.47 | 15.01 / 15.48 |
| 10 ms | 94.70 / 94.20 | 14.35 / 15.01 | 15.27 / 94.41 | 14.64 / 14.59 |
| 25 ms | 92.67 / 92.23 | 16.40 / 15.51 | 95.34 / 94.56 | 15.62 / 16.27 |
| 50 ms | 88.73 / 75.88 | 17.90 / 18.16 | 90.30 / 93.47 | 16.33 / 17.91 |
| 100 ms | 44.74 / 44.25 | 17.20 / 17.60 | 26.24 / 49.40 | 16.20 / 16.56 |

Candidate cells contain 24 samples each: 480 replacements total. Minimal baseline
cells contain 24 samples; full baseline cells contain 12 because the later
full guarded startup failed its requested-engine assertion. Flite's creation
wait expired while other helper creations took unusually long. The worker
correctly spoke through eSpeak fallback. That failure and all partial measurements
are retained; fallback is not counted as Flite qualification. The candidate's
sixteen Flite helpers across main/notification workers each started once, with no Flite helper restart
or cancellation watchdog timeout during these runs.

This identifies an avoidable cancellation wait and removes the observed practical
penalty. It does not fully explain why the earlier guarded blocks encountered the
wait more often. Controlled phases also made ordinary owners wait, so the original
33/71 ms difference should not be described as fixed transport overhead. The brief
initial focused Emacs test ran during part of the candidate phase run; these are
large median improvements, not isolated tail-latency certification.

## Foreground handover

For an ordinary local restart from negotiated Omnivox, the new foreground receives
capabilities, inventory, routing and voice registration before old foreground
retirement. Preparation is bounded by the existing two-second readiness wait;
failure or quit retires the unpublished candidate and preserves old foreground.
Global foreground ownership remains on the old process throughout preparation.
After handover, cached observations and readiness hooks are published without
repeating the control requests. Notification startup configures its own lane.
Managed activation retains its separate preparation, and remote/non-Omnivox
compatibility paths remain covered by tests.

A fresh compiled client submits source-tracked speech to both current lanes every
25 ms when that lane has no outstanding request. The gap is the interval from the
last observed old source start to the first new source start. It includes sampling
cadence, retirement, scheduling and synthesis; it is not a direct measurement of
acoustic silence or an exact admission blackout.

Matched ABBA runs use the same candidate Windows executable and companion payloads,
with the old compiled `tts-initialize` implementation loaded explicitly for the
baseline. There are eight restarts per variant. An earlier pilot ran during a
Linux build and is retained separately, excluded from this comparison.

| Measurement | Previous client | Prepared foreground |
| --- | --- | --- |
| Main source gap, median (range) | 1,473 ms (1,338–2,085) | 79 ms (54–113) |
| Notification source gap, median (range) | 111 ms (87–130) | 102 ms (84–135) |
| Entire restart, median (range) | 3,588 ms (2,827–4,636) | 3,744 ms (2,914–4,750) |
| New main source relative to operation completion | 79 ms after, median | 1,440 ms before, median |

Four additional candidate restarts deliberately pause notification startup for two
seconds. Main source gap remains 58 ms median (54–92), and new foreground speaks
3,441 ms before operation completion. The operation itself takes 5,724 ms median.
Old speech remains available during preparation in every run. These observations
support independent foreground availability, not a claim that total restart became
faster. Optional discovery and native initialization remain future startup costs.

Four additional restarts each select Eloquence and DECtalk, with all eight engines
enabled. Every observed old/new source event is checked against the selected
engine; fallback cannot satisfy the check.

| Selected engine | Main source gap median (range) | New main first source after restart command, median | Entire two-lane restart, median (range) |
| --- | --- | --- | --- |
| Eloquence | 70 ms (55–99) | 2,312 ms | 3,842 ms (3,189–5,767) |
| DECtalk | 134 ms (102–138) | 2,656 ms | 4,376 ms (3,729–4,969) |

Old foreground remains usable during replacement preparation. These operation
times include all enabled engines and both lanes; they are not timings of a
standalone vendor reset or helper recreation. Four observations establish only
the measured range, not tail latency. The initial Eloquence fixture failed because
its newly added engine field was not retained in the shared sample record. The
fixture was corrected and both engines rerun into new directories; the initial
raw output and fixture are retained separately.

## Further qualification

Four alternating blocks per path repeat character speech during real reload and
simultaneous Flite notification speech with all eight engines available. Every
request asserts the selected actual engine, successful source completion and an
applied reload. No warmup or fallback is counted as a measured selected-engine
request. Values are milliseconds; p95 is the empirical nearest-rank observation.

| Engine | Ordinary median / p95 | Guarded median / p95 | Samples per path |
| --- | --- | --- | --- |
| espeak | 2.95 / 4.83 | 2.78 / 4.94 | 160 |
| flite | 5.70 / 8.56 | 5.72 / 9.01 | 160 |
| winrt | 130.30 / 146.78 | 132.15 / 148.73 | 640 |
| eloquence | 5.27 / 6.96 | 5.33 / 6.90 | 160 |
| dectalk | 4.06 / 5.11 | 3.89 / 5.15 | 160 |
| rutts | 3.73 / 4.79 | 3.62 / 4.82 | 160 |
| tgspeechbox | 13.04 / 16.92 | 13.08 / 16.75 | 160 |
| rhvoice | 9.79 / 12.98 | 9.56 / 12.51 | 160 |

All observed guarded p95 differences are within the proposed +5 ms comparison
criterion in this workload. WinRT's larger sample now supports the matched
comparison, but its absolute timings differ from the previous report: ordinary
block medians are 125.2–134.9 ms and guarded blocks 129.6–133.6 ms. This does not
establish a universal tail bound or matched-release acoustic parity. The current
speech transport remains direct pipes; control bridges carry lifecycle operations.
Other engines have not inherited Flite's cancellation exception. Qualification
of any further exception must include actual cancellation, helper retention,
replacement latency and failure behaviour for that engine.

The Linux build supplied by `make build` also passes 120 real Flite replacements
through ordinary and guarded paths, with reload and notification speech. These
are functional native checks, not a matched Linux performance comparison.

| Failure or interruption | Observed protection | Evidence |
| --- | --- | --- |
| Flite keyed replacement | Old dispatch cannot complete as current speech; new source completes through Flite; helper remains loaded. | Windows phase matrix and Linux native probe. |
| Native call remains blocked after cancellation | Slot remains owned until actual completion; another acquisition is rejected. | Buffered and progressive regression tests, Linux and native Windows. |
| Foreground preparation errors or is quit | Unpublished candidate retires; old foreground and notification remain live. | ERT regressions. |
| Notification startup is slow | New foreground speaks throughout the extra wait. | Four native compiled-client restarts with a two-second delay. |
| Notification startup fails | Foreground is already configured and remains usable. | ERT readiness and lifecycle checks. |

Fresh compiled private profiles pass reload, restart, retained foreground recovery,
minimal eSpeak emergency startup and return to saved settings for all eight engines.
Both lanes retain their Windows guard and the healthy notification process survives
foreground recovery. These runs use null output and assert the actual selected
engine, not just successful process launch.

Validation also includes locked Linux workspace tests and all-target workspace
Clippy with Piper, Windows CLI all-target Clippy, eighteen native Windows isolation
tests, the focused Emacs readiness/recovery checks (74 passes, two native/graphical
skips), fresh byte-code/preflight and documentation gates. Full Emacs results are
4,435 expected, 17 failures and 238 skips out of 4,690 tests. The failures exactly
match the retained unchanged baseline: fourteen Magit dependency failures, the
stale generated aural reference and two existing manual assertions.

Failed Linux build attempts are retained: missing explicit Piper inputs and native
eSpeak path-length failures in long build directories. Verified existing Piper
inputs and the previously qualified short `/home/bart/tmp/ovlw-check` target pass.
No native source, dependency, historical artifact or unrelated checkout was altered
to repair those environmental failures.

## Provenance and limitations

Omnivox implementation: `7b86862`. Emacsvox implementation: `06a8722a1`.
The Windows runtime is `5c330933b3325f9d`, executable SHA-256
`bac425201a1bb5d175e34a01ddf1b6f3775172e91fb6ac42905060ede094e67c`.
It was fully staged from `10f7605` plus the retained implementation patch; subsequent
source changes before the implementation commit are formatting and unit tests.
Every companion executable and runtime DLL is byte-identical to the previous
`ff90ae379e43e184` payload. Main-only staging was rejected because the previous
stage preceded a helper README change; full staging was used.

The [evidence pack](data/2026-11-10-replacement-and-handover/README.md) retains raw
samples, failures, fixtures, source patches, provenance, checks and summaries.
The [private launcher](data/2026-11-10-replacement-and-handover/listening/start.py)
uses separate settings and the pinned executable; `--engine flite` selects the
qualified prototype and `--batch-check` uses null output. Installed speech,
maintenance main and historical release/session evidence are untouched.

Listening, physical output interruption, acoustic timing, native macOS and Windows
ARM remain unverified here. Windows Piper and engines needing additional child
creation remain outside the opt-in guard's current qualification. Their ordinary
launch path is retained. This is a private prototype, not a release or permission
to replace installed speech.
