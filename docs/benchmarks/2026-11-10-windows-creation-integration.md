# Windows guarded speech: integrated prototype

The experimental branch now permits opt-in Windows guarded startup for explicit
eSpeak/Flite profiles under [ADR 0019](../adr/0019-windows-native-creation-ownership.md).
The guard retains actual native ownership before execution; speech and helper PCM
stay on direct pipes. Ordinary launch and lightweight reload remain available for
other engines. This is a private prototype, not release or everyday-engine acceptance.

The [component milestone](2026-11-10-windows-owned-creation.md) remains historical
evidence. This report covers its integration, a real pipe compatibility defect,
confirmed helper-launch rejection, native recovery and matched speech measurements.

## Changes and failure coverage

A helper creation failure before returning a process or transferring handles can
now produce a correlated rejection. The guard checks its owned set, and the worker
retains the creation channel and existing speech. Unknown ownership or uncertain
handoff still retires the receiving worker. Six native component cases cover
pending creation, worker loss during creation, lost offer/acknowledgement, duplicate
or changed launch, unaccounted job activity, and repeated confirmed launch failure.

The first integrated candidate hung during Flite initialization. Its anonymous
pipes were synchronous at both ends, but Rust's `ChildStdin`/`ChildStdout` handle
conversion requires asynchronous worker handles. The corrected pair uses a
synchronous helper end and overlapped worker end; the guard transfers ownership
without forwarding audio. A native regression test transfers 256 KiB in each
direction and checks EOF, exercising data larger than the pipe capacity.
[Rust child-stream contract](https://doc.rust-lang.org/std/process/struct.ChildStdin.html).

The first corrected pair also reused the 16 KiB control-buffer size. The final
helper pipes request 64 KiB, matching the pinned Rust child-pipe implementation;
control framing stays independently bounded. This removes an accidental transport
difference and the replacement penalty observed in the initial matched follow-up.
It is not proof that buffering alone explains every timing difference.
[Pinned Rust source](https://raw.githubusercontent.com/rust-lang/rust/1.97.1/library/std/src/sys/process/windows/child_pipe.rs).

The native fixture retains actual process objects before injecting faults. It
corrupts private settings/host records and denies session writes; claims and
management bytes must stay unchanged. A separate notification lane speaks after
every case. The matrix covers:

| Fault | Required observed outcome |
| --- | --- |
| Owner killed or paused | Surviving guard confirms actual worker/helper and forwarding-I/O completion; minimal eSpeak starts. |
| Speech input or output blocked | Independent retirement completes; minimal speech starts. |
| Worker exits while its helper survives | Actual helper completion precedes cleanup proof; minimal speech starts. |
| Lifecycle bridge disconnected | Existing speech continues; reconnect to the same authority succeeds. |
| Helper killed three times | Configured eSpeak fallback speaks; an explicit recovery probe restores Flite in the same worker each time. |
| Status output saturated | Worker/helper exit despite held replies; drained final reply carries real cleanup proof. |
| Guard lost | Replacement remains blocked without proof; healthy notification speech continues. |

The helper-crash check explicitly configures eSpeak fallback. A route restricted
to Flite can fail its next request after that helper dies; the new guard does not
silently override routing policy. Minimal recovery uses bundled eSpeak only after
old native ownership is confirmed. Guard loss remains an acknowledged limitation.

## Matched timing

Windows x64 runs through WSL with the same final optimized executable and Python
client for ordinary and guarded owners. Null output isolates source processing;
it does not measure the physical device or acoustic onset. Four alternating
blocks contain 80 character requests per engine/variant, each submitted after
reload transmission while notification speech runs. Protocol figures are separate
capability-query round trips. Startup figures include two serial lane starts and
their first usable source speech, not a measured restart blackout.

| Engine/path | Character source median | Empirical p95 | Protocol median | Two-lane startup median |
| --- | ---: | ---: | ---: | ---: |
| eSpeak ordinary | 3.26 ms | 5.36 ms | 0.97 ms | 834 ms |
| eSpeak guarded | 3.06 ms | 4.37 ms | 0.98 ms | 1,442 ms |
| Flite ordinary | 5.40 ms | 8.92 ms | 1.04 ms | 906 ms |
| Flite guarded | 5.72 ms | 8.58 ms | 1.02 ms | 1,494 ms |

There are 320 character samples per row. Block medians span 2.99–3.63/2.84–3.25 ms
for ordinary/guarded eSpeak and 5.05–5.80/5.35–6.11 ms for Flite. These differences
stay below the plan's proposed investigation threshold. The empirical p95 is
descriptive; this is insufficient for a strong tail-latency or acoustic claim.
This matched comparison isolates the guard. It does not replace the retained
1.18.0 baseline comparisons or establish every client/engine's release equivalence.

Stop and same-key replacement also run during reload and notification speech:
20 of each per engine/variant. Every old utterance terminates without completion,
and each replacement completes. Final stop-terminal medians are about 2.1/2.2 ms
for eSpeak and 1.4/1.5 ms for Flite, ordinary/guarded respectively. Source start
after Stop is about 4.7/4.5 ms and 8.9/8.3 ms.

Long-utterance Flite replacement varies substantially. The first 16 KiB comparison
showed ordinary/guarded source medians of 75.7/91.0 ms; an alternating follow-up
with 80 replacements each showed 25.9/32.3 ms. After matching 64 KiB capacity, the
same follow-up showed 23.5/19.5 ms, with block medians spanning 18.1–28.5/16.5–23.0 ms.
The final separate 20-request interaction check showed 93.6/20.5 ms. Retain all
runs: they support removal of the observed guard penalty, not a general claim
that the guard improves replacement or that replacement variability is solved.
Stop, replacement, source completion and acoustic silence are distinct observations.

## Client, checks and handoff

Compiled Emacsvox at `b83c43aad`, explicitly loaded from its isolated worktree,
passes recovery after a paused/killed WSL connector, repetition, lost guard reply
and bridge reconnection. Native owner faults are established by the separate
PowerShell matrix, not inferred from stopping a WSL connector. Preparation/reload
checks keep both existing lanes speaking. The private profile exercises actual
reload, `tts-restart`, `tts-recover`, minimal emergency startup and a return to
saved settings, checking the selected engine and notification identity.

Windows checks include 13 pipe tests, six owned-creation tests, correlated-rejection
coverage and three explicit-runtime bootstrap/scope tests. The prior 60 native
helper-engine tests still cover unchanged TTS code. Locked Windows all-target
Clippy/test compilation and Linux workspace tests/Clippy pass, including the CLI
Piper feature. Full private staging verifies ordinary eSpeak, Flite, RuTTS and
TGSpeechBox; subsequent main-only staging uses its accepted change guard.

The final native matrix and private-profile reruns pass with the final payload;
the raw pack retains their outputs. No device/listening or acoustic check has been performed.
Use the [isolated launcher](data/2026-11-10-windows-creation-integration/listening/start.py)
with `--engine flite` or `--engine espeak`
for a fresh separate profile; `--batch-check` uses null output. It verifies the
pinned executable hash and client byte-code before startup. Installed speech,
personal settings, main and the release baseline stay unchanged.

## Provenance and failed attempts

The [raw pack](data/2026-11-10-windows-creation-integration/SHA256SUMS) contains
commands, fixtures, all failed runs, source patches against `f6a6045`, native
outputs, build provenance and checks. Archives preserve full machine output;
executables and debug DLLs are identified by hashes rather than duplicated.
The final private runtime is `d2fbb22badb94e02`, executable SHA-256
`ea7d33d13c7777cfda1c8989731d10cc8b3ecc5d25feb56caae4343eb64f508c`.
It is staged under Windows Temp. Its helpers come from the fully rebuilt private
runtime `28cf0d81c4e6ff81`; main-only verification records this reuse.

Retained fixture failures are distinct from product failures:

- After the synchronous-pipe product defect was corrected, a status-pressure
  fixture tried to reconnect after receiving proof and closing the last owner.
  Correct guard exit made that assertion invalid; the revised test consumes the
  already verified proof.
- The first helper-crash fixture assumed immediate Flite recovery with no fallback
  or recovery request. Its revision checks configured fallback and explicit probe.
  A later decreasing dispatch generation was correctly cancelled; the fixture
  now uses increasing generations.
- Batch Emacs inherited default SIGPIPE and exited during an intentional broken
  connection. Interactive Emacs ignores SIGPIPE; the batch harness now preserves
  Python's ignored disposition to match interactive behavior. The local Emacs
  `src/sysdep.c` documents that distinction. No Lisp product change was needed.
- The first profile test expected `applied` for unchanged preferences. The corrected
  fixture changes its private chunk setting before reload.

All these failed outputs remain available. Ordinary 1.18.0 speech, accepted
architectural decisions and historical evidence were not rewritten.

Remaining work is listening acceptance, actual everyday-engine coverage,
startup/recovery cost, native macOS and recovery after losing all authorities.
The guard remains opt-in. A qualified eSpeak/Flite prototype is a useful milestone,
not completion of the broader portability and availability goal.
