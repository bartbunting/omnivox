# Recovery after a lost native retirement reply

The ordinary recovery gap found in the
[preceding client delivery](2026-11-10-retained-recovery.md) is now closed when
the owner's real durable receipt remains intact. This increment keeps direct
speech and independent preference reload. It introduces no resident process,
forwarding hop or management write on ordinary speech or reload.

## Behaviour and safety boundary

The native owner persists its exact retirement receipt after its worker tree and
pipe readers finish, then replies and exits. Dropping that final reply previously
left the client unable to distinguish completed cleanup from owner loss.
The local management service now negotiates `retirement_receipts_version: 1`
and verifies that existing proof through `retirement-status`. The
[contract](../reference/engine-startup-snapshot.md#reconcile-an-existing-retirement-receipt)
defines worker/path/hash binding, bounds and failure results.

The paired client retains the original launcher, working directory and environment
privately on each owned process. Reconciliation therefore targets the original
host even if current settings change. It accepts only a matching worker/digest
with confirmed retirement; old hosts and clients lacking retained context remain
unsupported. A later owner still verifies its executable, assets and complete
startup and publishes its own retention record before native initialization.
Receipt inspection does not need to repeat those asset checks to establish past
cleanup.

Prepared records, absent receipts, malformed or changed data, another worker or
host path, busy storage and missing locks cannot establish retirement. Inspection
neither recreates the retention lock nor rewrites history, drops claims or signals
a saved PID. The existing proof relies on the private host namespace and owner
writer; it does not authenticate arbitrary external rewrites of that storage.
This is ordinary recorded-cleanup reconciliation under ADR 0007, with unchanged
native ownership, asset and cancellation rules. It does not require a new ADR.

## Native checks and observed availability

Fresh source and compiled Linux clients recover after losing the actual final
owner reply. Real Flite and Piper/Lessac medium synthesis then completes. Fresh
compiled WSL Emacs also recovers through the native Windows Flite runtime.
The fixture verifies actual engine IDs and loaded Lisp paths. Malformed current
settings remain present throughout; retained startup supplies recovery.

Separate Linux and Windows fault cases damage the real receipt after its native
writer finishes. Recovery and retry refuse replacement, the damaged bytes remain
unchanged, and notification speech completes. The original receipt bytes are
preserved as test evidence, never restored or substituted as production proof.

The final fixture submits a notification during explicit foreground recovery and
requires its completion before that operation returns. The same notification
owner and acknowledged voice registration survive. Observations from these runs:

| Platform and engine | Notification submission / completion within recovery | Foreground recovery operation | Recovery plus completed main speech | Lost-reply recovery plus completed main speech |
| --- | --- | --- | --- | --- |
| Linux Flite | 10.204 / 43.113 ms | 125.166 ms | 160.941 ms | 243.594 ms |
| Linux Piper/Lessac medium | 10.517 / 131.953 ms | 581.301 ms | 692.331 ms | 752.381 ms |
| WSL to native Windows Flite | 10.165 / 51.693 ms | 679.046 ms | 708.899 ms | 1292.739 ms |

These are individual functional observations with null output, not acoustic
onset, blackout distributions or matched release performance comparisons. Other
runs, including higher timings under concurrent work, remain in the archive.
The notification timestamps demonstrate actual client-observed speech completion
inside recovery, not merely a live process before and afterward.

The preceding host-preference fixture also passed on the new Linux payload:
50 two-worker updates, 100 speech requests, rejected native-scope edits and
malformed files, preserved owners and foreground reload with the secondary absent.
Earlier matched release latency evidence remains unchanged; no new steady-state
speedup is claimed here.

## Verification and provenance

Locked Rust workspace tests passed: 1,211 tests, two ignored across 38 suites.
Relevant CLI/TTS Clippy with Piper and formatting passed. Four added Rust tests
cover exact, prepared, absent, partial, changed, foreign and linked evidence,
lock contention and refusal to recreate a missing lock. The initial workspace
attempt lacked the prepared Piper input path; the retained configured rerun
passed without changing dependencies.

Eleven focused client regressions passed. The full client core suite passed
1,057 tests with 18 conditional skips. Incremental byte-code compilation and
preflight passed after the previous slice's full rebuild. Source and compiled
native runs verified their actual loaded paths. Headers, generated documentation
and final client/paired documentation gates passed. An initial isolated unit
invocation omitted the voice module; the fixture now requires it explicitly and
its passing rerun is retained.

`make build` staged the complete Linux payload. Native Windows full development
staging rebuilt and verified the runtime in a separate temporary installation;
its native helper inventory checks passed. Installed speech and live Emacs were
not changed. Linux executable SHA-256:
`d7b2b28c9ecbcff96945466270a768a40665890a97c08e80c458b635830294d9`.
Windows executable SHA-256:
`cf085ab8574c81e5ba1262aeb304bf98a42c1a668fa409ed16df3552da78bd99`, runtime
`b5f4a748c232ba16`.

The [evidence pack](data/2026-11-10-retirement-receipts/README.md) retains source
patches against Omnivox `6d117c9` and Emacsvox `4479ed37c`, fixtures, checksums,
raw logs, the damaged-receipt inputs, payload manifests and staging provenance.
Earlier evidence packs and payloads are preserved. Fixture assertions were
extended to require concurrent notification completion; earlier functional runs
are labelled separately and are not claimed to be identical timing workloads.

## Remaining work

Keep this smaller design: direct owned speech, independent coherent preferences,
and explicit lifecycle recovery. Missing cleanup, damaged host/startup records or
unwritable management storage still block ordinary restoration. Reading a valid
receipt closes reply ambiguity; it does not manufacture native authority after
owner loss. The emergency-start boundary remains a separate pending decision in
the [plan](../plans/lightweight-preference-reload.md#next-recovery-experiment-and-proposed-emergency-boundary).
Native macOS, native Windows Emacs, acoustic onset and Stop-to-silence remain
unqualified here. This is development evidence, not a release or complete
qualification of the broad always-recover goal.
