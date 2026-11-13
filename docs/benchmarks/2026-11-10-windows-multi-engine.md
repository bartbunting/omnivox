# Windows guarded speech: multiple engines and startup cost

The opt-in Windows prototype now supports eight explicit engines: eSpeak, Flite,
WinRT, Eloquence, DECtalk, RuTTS, TGSpeechBox and RHVoice. Six helpers can coexist
in each worker, with independent retained native ownership and helper recovery.
Speech and PCM remain on direct pipes. This extends the
[initial eSpeak/Flite integration](2026-11-10-windows-creation-integration.md)
under [ADR 0019](../adr/0019-windows-native-creation-ownership.md); it does not
change that decision, qualify every platform, or deploy installed speech.
The expanded workload also exposes a repeatable Flite replacement penalty that
must be resolved before broader adoption; functional success is not performance
acceptance.

Overlapping the client control bridge with ordinary owner preparation reduces the
matched full-restart median from **3.55 to 3.18 seconds**. Continuous source probes
also expose an unresolved foreground handover gap: the client retires old main
speech before finishing notification initialization and final voice setup.
Faster preparation alone does not remove that gap. Alongside the Flite timing
investigation, the next startup experiment should make foreground configuration
usable before this handover, independently of notification startup, while retaining
native cleanup and cancellation rules.

## Implementation and native failures

The Windows guard retains at most eight helper slots and one pending creation or
handoff. Each slot corresponds to an enabled, available helper in the validated
startup snapshot. Actual completion frees a slot; acknowledgements identify the
specific offered helper. A confirmed pre-process launch rejection leaves existing
helpers and worker usable. Uncertain transfer or an occupied-slot protocol error
still closes the worker safely. The guard's creation lock never handles normal
speech, PCM or ordinary preference reload.

Two real compatibility defects appeared when extending coverage:

- The C# console helpers hung under restricted child creation. Building them as
  Windows-subsystem executables removes their console-host requirement while
  retaining redirected standard I/O, their existing protocols and separate GPL
  executables. An attempted process-flag substitution prevented worker readiness
  and was discarded; child restrictions were not relaxed.
- Default staged helper paths use verbatim Windows names. Passing these unchanged
  as `lpApplicationName` made .NET Framework reject its configuration path when
  initializing its JSON serializer. The guard now follows the pinned Rust
  `Command` boundary: shorten a verbatim drive/UNC path only when a bounded
  `GetFullPathNameW` check gives exactly the same ordinary path. Keep long paths,
  different normalization and native UTF-16 units intact. Retained launch identity,
  argv, environment, executable checks and asset verification stay unchanged.
  [Rust process implementation](https://raw.githubusercontent.com/rust-lang/rust/1.97.1/library/std/src/sys/process/windows.rs),
  [Rust path conversion](https://raw.githubusercontent.com/rust-lang/rust/1.97.1/library/std/src/sys/args/windows.rs).

The final native matrix uses all eight engines and retains actual handles for six
helpers before fault injection. PID discovery is used only to acquire and validate
those handles; fault handling and completion checks use the retained objects.
Private configuration/host records are damaged and session writes denied. Their
management bytes remain unchanged, and a separate notification lane remains usable.

| Fault | Observed outcome |
| --- | --- |
| Owner killed or paused | Guard retires all actual worker/helper objects and forwarding I/O; minimal speech starts. |
| Speech input or output blocked | Independent retirement completes and minimal speech starts. |
| Worker exits with suspended surviving helper | Actual helper completion precedes proof and replacement. |
| Lifecycle bridge disconnects | Existing speech continues; reconnection to the same authority succeeds. |
| Each helper killed | Configured eSpeak fallback speaks; explicit recovery restores that engine in the same worker, preserving siblings. Flite repeats three times; eight helper recoveries in total. |
| Status output held | Native retirement completes despite response pressure; draining returns actual proof. |
| Guard lost | Replacement remains unavailable without proof; notification speech continues. |

These are nine matrix cases. Guard loss remains a real availability limitation,
not a successful emergency recovery. No ownership records are deleted and no
missing cleanup evidence is invented. Piper, MBROLA and external wrappers remain
outside guarded Windows qualification and available through ordinary launch.
Piper requires a supplied companion/model; child-creating wrappers require an
explicit supported creation path. Unsupported guarded configurations are rejected
before replacement; preferred engines are not silently disabled.

## Speech during reload

The same optimized executable and Python client compare ordinary and guarded
owners on native Windows x64 through WSL. Both initialize all eight engines, with
six helpers per worker. Each character request follows reload transmission while
a notification request runs. Four alternating blocks give 160 character samples
per engine/path. Every reload changes the tested policy and reports `applied`;
every utterance checks the requested engine and completion.

Times below are client-observed **source-start** latency with null output, in ms.
They are neither a protocol round trip, time to first PCM, nor acoustic onset.

| Engine | Ordinary median | Guarded median | Ordinary empirical p95 | Guarded empirical p95 |
| --- | ---: | ---: | ---: | ---: |
| eSpeak | 3.48 | 3.12 | 5.83 | 4.68 |
| Flite | 6.21 | 5.84 | 10.24 | 9.14 |
| WinRT | 103.22 | 106.50 | 131.50 | 139.09 |
| Eloquence | 4.91 | 4.71 | 6.94 | 6.20 |
| DECtalk | 4.10 | 3.92 | 5.89 | 5.25 |
| RuTTS | 3.68 | 3.43 | 4.61 | 4.40 |
| TGSpeechBox | 11.66 | 11.40 | 15.27 | 14.81 |
| RHVoice | 8.52 | 8.44 | 11.80 | 12.15 |

WinRT exceeds the plan's proposed +2 ms median/+5 ms empirical-p95 screening
threshold against this ordinary candidate. Its block median ranges overlap:
99.18–110.43 ms ordinary and 99.36–110.82 ms guarded. Per-request attribution shows
native synthesis medians of 60.41/62.34 ms, while the median of each request's
source latency minus its native synthesis duration is 42.35/42.35 ms. This
supports native variation as a contributor; it does not prove tail equivalence
or excuse a persistent regression. Do not add or subtract aggregate medians to
construct a missing latency component. The guard introduces no speech-forwarding
hop in either path.

This comparison isolates guarded ownership, not equivalence to every 1.18.0
engine/client combination. Retain the
[matched release evidence](2026-11-10-lightweight-reload.md) and
[resident comparison](2026-11-10-resident-reload.md). Empirical percentiles here
are descriptive, with insufficient samples for a strong tail-latency claim.

Twenty Stop and twenty same-key replacement trials per engine/path exercise long
speech during actual reload and notifications. All old utterances terminate as
cancelled and all replacements complete with the requested engine. WinRT's
Stop-terminal medians are 117.8/122.7 ms ordinary/guarded; most other engines are
much quicker. Cancellation status does not measure acoustic silence.

The initial replacement medians were 35.2/82.5 ms for Flite, 97.8/144.6 ms for
DECtalk and 8.9/17.0 ms for RuTTS. Eloquence Stop was 2.1/5.7 ms. A longer
follow-up alternates ordinary/guarded order across four blocks of 40 trials:

| Interaction | Ordinary median | Guarded median | Samples per path |
| --- | ---: | ---: | ---: |
| Flite replacement source | 33.31 ms | 71.42 ms | 160 |
| DECtalk replacement source | 125.18 ms | 126.83 ms | 160 |
| RuTTS replacement source | 10.08 ms | 11.59 ms | 160 |
| Eloquence Stop terminal | 2.29 ms | 2.25 ms | 160 |

**Flite remains a performance blocker for broader adoption.** Its ordinary block
medians span 26.2–38.2 ms; guarded blocks are 77.2, 31.4, 77.9 and 73.3 ms.
Correlated worker lifecycle records put most of the extra time before the new
synthesis worker starts, waiting for previous synthesis to finish. The reason the
guarded workload reaches that longer wait more often is not established. This is
not explained away as noise, nor evidence of an added guard transport hop.
A bounded next probe should vary interruption phase with identical text/chunking
and compare eSpeak/Flite-only versus all-eight-engine profiles, retaining queue,
native-synthesis and source timestamps. No cancellation or integrity protection
was weakened. Earlier single-helper results remain valid historical observations,
not proof that this larger workload has no replacement regression.

## Restart and availability

Compiled Emacs 31.0.90 loads either unchanged `tts-speak` from `b83c43aad` or the
new client, with other Lisp and the final executable identical. The change starts
an authority-free waiting bridge while the owner prepares, then attaches it only
after validating the descriptor. Error and quit discard an unused bridge and
replacement while preserving old speech. Emergency preparation already supported
this overlap; ordinary startup now uses it too.

Four fresh profiles run baseline/candidate/candidate/baseline, each with four
restarts, all engines enabled and Flite selected. The eight samples per variant
show:

| Measurement | Previous client | Overlapped client |
| --- | ---: | ---: |
| Restart operation median | 3,549 ms | 3,181 ms |
| Operation range | 2,952–4,134 ms | 2,531–3,736 ms |
| First source after invoking restart, median | 3,636 ms | 3,268 ms |
| Bridge connect median, including initialization | 165 ms | 1.7 ms |

The roughly 368 ms aggregate reduction is consistent with overlapping bridge
startup twice. Native initialization also varies between runs; this small sample
is not a precise causal decomposition. Old speech remains alive during preparation,
so the full operation interval must not be labelled speech blackout.

A separate probe submits short speech every 25 ms when each lane is ready, with
at most one pending request per lane, across four restarts per client. Both old
lanes produce source events during preparation; new lanes subsequently speak.
All requests complete or are cancelled at retirement. The interval between the
last old and first new **observed source event** is:

| Lane | Previous client range | Overlapped client range |
| --- | ---: | ---: |
| Foreground | 1,511–1,711 ms | 1,359–1,615 ms |
| Notification | 91–112 ms | 109–118 ms |

These intervals include scheduling, sampling and synthesis; they bound an observed
handover gap, not acoustic silence. The client code explains the long foreground
interval: `tts-initialize` publishes the replacement after retiring the old main,
then starts notifications, then calls `voice-setup`. The replacement's client
registration is not usable throughout that wait. This also means one slow
notification startup can delay foreground usability. Removing this dependency is
a better next experiment than weakening ownership verification.

Opt-in Rust startup spans in the candidate show per-lane medians of approximately
14 ms preparing the engine snapshot, 2.6 ms saving startup, 14 ms verifying startup,
74 ms launching the guard, 2.9 ms waiting for its readiness, 93 ms starting the
worker, 11 ms sending startup, and 354 ms initializing eight engines. The outer
client owner request is about 755 ms and engine preparation about 681 ms. These
are nested or differently bounded observations: **do not sum them**. Internal
guard stderr is discarded, so the worker-start span does not separate its internal
verification from process creation. Repeated hashing and durable startup writes
are measured management costs; they are not on the speech path and do not explain
the whole restart. No integrity checks were removed to obtain the improvement.

## Verification and prototype

All eight selected engines pass fresh compiled full-profile checks for ordinary
reload, restart, retained recovery, emergency eSpeak and return to saved settings.
Recovery preserves the notification process. These checks use null output;
listening, a real output-device interruption test and acoustic measurement remain
outstanding. The [private launcher](data/2026-11-10-windows-multi-engine/listening/start.py)
accepts `--engine` with any of the eight names above; `--batch-check` selects null
output. It pins the executable hash and checks current client byte-code before
starting an isolated profile. User-supplied native runtimes remain external.

Other checks completed:

- Locked Linux workspace tests and all-target workspace Clippy, including Piper.
- Locked Windows CLI/TTS all-target Clippy and test compilation; 28 native guard
  tests, including explicit-runtime cases. Multiple-helper rejection, retained
  completion, stale acknowledgement, pipe pressure/EOF, path compatibility and
  bounded handle counts are covered. Counts settle at 139/140 after warmup in the
  repeated ownership test; this is not an RSS/CPU bound measurement.
- Twelve Windows helper source checks and complete pinned development staging,
  followed by guard-approved main-only staging. The rejected main-only attempt
  after helper source changes remains in the evidence; full staging was used.
- Emacs focused recovery tests: 35/35; incremental byte-code build and preflight;
  documentation release check; actual compiled private-profile execution.

The full Emacs suite is **not green**: 4,688 tests give 4,433 expected results,
17 failures and 238 skips. An unchanged `b83c43aad` checkout gives exactly the same
17 failures (4,432 expected, 4,687 total): fourteen Magit dependency failures,
a stale generated aural reference and two existing manual assertions. The added
regression passes. Raw logs and the exact comparison are retained; unrelated
failures were not changed to make this slice appear clean.

## Provenance and limits

Omnivox implementation commits are `5a892fa` (console-free helper builds) and
`dce3f3b` (multiple helpers, application paths and startup spans). Paired Emacsvox
commit `d44a2c9a4` overlaps bridge startup. Final runtime `ff90ae379e43e184` has
executable SHA-256
`0e1d2b1e7bed15bcea8c45188eafa55c542aa315bc1fa676da3f4b6a070baca8`.
Its exact recorded source patch against `5a892fa` reconstructs the committed
executable source; the evidence checks that equivalence. Helper payloads come from
full build `a4167ed7873f1a5b`. Toolchains, payload manifests and external RHVoice
hashes are recorded. Existing Eloquence/DECtalk runtimes are not redistributed.

The [evidence pack](data/2026-11-10-windows-multi-engine/README.md) retains raw data,
fixtures, intermediate failures, source patches, provenance and check logs.
Failed attempts include the console helper hang, rejected process flags, .NET
verbatim-path failure and fixture corrections: an unchanged punctuation policy,
a null CIM path, a missing queue attachment in a unit fixture and a timer wait
that returned on output before observing new speech. Original outputs survive.
Initial engine discovery missed the locally staged RHVoice configuration; final
qualification uses its explicitly recorded library/data paths.

This establishes Windows x64 native ownership and WSL client integration for eight
engines, with bounded multi-helper recovery and measured ordinary-startup overlap.
It does not establish native macOS or Windows ARM qualification, listening
acceptance, recovery after losing every authority, all-engine performance parity,
or a finished release. Installed speech, maintenance main, release 1.18.0 and the
retained session experiment remain separate. The guard remains opt-in.
