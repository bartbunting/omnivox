# Lightweight punctuation reload investigation — 2026-11-10

## Verdict

Proceed with independent preference reload on ordinary owned workers, selectively
reusing the session experiment's reliability protections. The first punctuation
slice preserves ordinary speech response and applies without restarting engines.
Neither a new speech forwarding layer nor global durable preference completion
was necessary. Finish a small supported reload interface before expanding settings;
do not finish managed-library Apply merely to deliver preference reload.

Confidence is high for this architectural direction and the bounded Linux
punctuation experiment, lower for production recovery and platform qualification.
This is an investigation result, not permission to deploy or a claim that every
failure can now recover. Evidence of a necessary cross-worker safety invariant,
maximum-size policies blocking Stop, or a simpler session implementation meeting
the same performance and independent availability criteria would change the verdict.

## Sources and isolation

The candidate is `experimental/lightweight-reload` from maintenance `f3d6eda`.
Release `v1.18.0` is `246e3b7`. The session comparator remains at `7879419` on
`experimental/session-reload`; its Emacsvox branch remains `a0404c398`.
The new Emacsvox source fixture is based on `master` at `fb916e4f3` in a separate
worktree. No merge, release, installed-runtime replacement or live Emacs restart
was performed. Existing working files and historical measurements were retained.

The Linux executable used for the matched measurements has SHA-256
`8b585283f4f8b20a726fcff6e33c355285323e1aa1e571cd9b8cb55a108dca6a`.
The [evidence directory](data/2026-11-10-lightweight-reload/) contains raw results,
probes, source patch, payload/input hashes, logs and reproduction notes. Early
fixture failures and the pre-revision-identity pilot are retained separately from
the final comparisons. Build wrappers supplied native payload data and notices.
The final capture-only Windows path correction produces Linux SHA-256
`ed2143e7f7aa1f55c44d44f84d56284d30d30c3086cc1a64b1106b52df608fa0`.
It leaves admission/playback unchanged. Final workspace and native functional
checks cover that correction; the latency matrix was not rerun under the new hash.
All 364 selected eSpeak data files match byte-for-byte between release and candidate.

Maintenance retains post-release helper-write hardening. Existing bounded queues,
progressive PCM, generation cancellation, output recovery, native helper isolation
and owned startup/retirement were reused. The session branch's later cleanup guards,
failure tests, immutable settings preparation and managed activation checks remain
valuable; they were not imported wholesale into this first slice.

## What changed

A private, opt-in Rust thread accepts and validates one punctuation candidate per
worker. Full capture uses the existing startup resolver without native engine
initialization. Only punctuation may differ; mixed edits reject. Installation swaps
an immutable policy at the existing request boundary. Native workers, helpers and
output objects survive. Foreground and notification installation results are
independent. Live policy revision is distinct from immutable native launch identity.

The [maintained plan](../plans/lightweight-preference-reload.md#private-experiment-contract)
records the experiment's contract and remaining delivery work. The implementation
adds one module and small CLI/admission hooks; it does not introduce a management
process, persistent journal, polling loop or public protocol capability.

Speech still follows client → ordinary owner → bounded worker admission → synthesis
→ host playback. Management parsing stays off that path. There are no repeated
executable hashes, inventory discovery, durable writes or cross-lane acknowledgements
on installation. Candidate capture still reads selected files, and policy hashing
still happens on admission: precomputing those hashes is a production requirement
before claiming behavior for maximum-size valid configurations.

## Matched speech measurements

All times below are client submission to receipt of the source-start event,
in milliseconds, using null output. They are neither first PCM nor acoustic onset.
Release, maintenance and candidate use Rust 1.97.1 release builds, the same Python
client, two owned workers, isolated minimal configuration and the same workload.
Eight rotated blocks give 1,024 measured requests per case and workload over eight
independent starts. Raw observations retain every outlier.

| Workload | Release median / p95 | Maintenance median / p95 | Candidate idle median / p95 | Candidate reload median / p95 |
| --- | --- | --- | --- | --- |
| Short ordered speech | 1.634 / 2.567 | 1.675 / 2.762 | 1.658 / 2.473 | 1.673 / 2.492 |
| Real palette letter command | 1.573 / 2.396 | 1.625 / 2.549 | 1.566 / 2.460 | 1.575 / 2.491 |
| Sequential replaceable speech | 21.829 / 22.441 | 21.800 / 22.441 | 21.811 / 22.517 | 21.821 / 22.537 |

These 12,288 requests all completed. The existing 20 ms replacement coalescing
window explains the different replacement baseline; it is not reload overhead.
Maximum observations remain approximately 152–153 ms for ordered/letter speech
and 172–173 ms for replacement across all four cases. They remain unexplained
outliers, not evidence of a new reload regression or an acceptable worst-case bound.
Empirical p95 and per-start variability are in `summary.json`; no p99 claim is made.

Those runs include requests outside the shorter management interval. A separate
timestamped comparison used 1,000 requests per case over five starts with sustained
reload and concurrent Windows compilation. Every reload-case request was submitted
inside the management run; 606 request-to-source intervals overlapped a management
RPC on that same worker.

| Timestamped subset | Count | Median | Empirical p95 |
| --- | --- | --- | --- |
| Idle under build load | 1,000 | 1.996 | 2.859 |
| During sustained reload under build load | 1,000 | 2.257 | 3.686 |
| Actually overlapping a same-worker RPC | 606 | 2.373 | 3.892 |

All completed. Idle per-start medians ranged 1.754–2.217 ms; reload medians ranged
2.007–3.042 ms. This intentionally loaded run is separate from the main comparison.
The smaller overlap subset is descriptive, not a sufficiently sampled tail claim.

## Settings cost, native engines and interruption

In the matched ordered workload, prepared per-worker installation had median
0.999 ms, empirical p95 1.637 ms and maximum 2.583 ms across 1,616 operations.
Full candidate capture had median 3.243 ms over eight observations. These are
different boundaries and must not be presented as a measured end-to-end sum.
Replacement coalescing can delay a safe installation boundary: its per-worker
operation p95 was 19.026 ms. Foreground speech did not wait for notification success.

The full capture-through-two-replies probe records 100 actual wall-clock operations
in `total-1/result.json`: median 7.496 ms, empirical p95 10.159 ms, maximum 15.688 ms.
Foreground completion alone had median 5.924 ms and p95 7.654 ms. This one-start
screen is descriptive, not a general tail guarantee. It measures capture and sequential independent installation
after editing the file; it excludes the edit/write itself. Engine reconstruction
and admission blackout are absent from this design. No failed valid speech request
or reload-induced native replacement was observed; null output cannot establish
an acoustic blackout bound.

Actual-engine assertions passed for Flite and Piper, each with 40 measured speech
requests during the trial and repeated reload. Median source response was 3.441 ms
for Flite and 42.075 ms for Piper; median prepared installation was 1.199 and
1.335 ms respectively. These are functional expensive-engine checks, not matched
cross-build tail comparisons. Piper used a copied existing 1.9.0 helper and the
Lessac medium model, with exact hashes retained; it does not qualify a new Piper
companion build. Native synthesis text length changed from 13 to 31 bytes for the
selected punctuation profile, independently confirming a real preparation change.

An independently paced test submitted 200 replacements at 5 ms intervals, including
a long initial passage, ten Stops, 40 simultaneous notifications and 300 reloads.
Every submitted dispatch received exactly one completed/cancelled terminal result,
the final dispatch completed, and subsequent speech succeeded. This exercises
offered load independently of reply speed. It does not replace physical
Stop-to-silence or sustained real-time device playback qualification.

## Availability and failure observations

| Injection/check | Observed behavior and completion condition |
| --- | --- |
| Malformed file, oversized frame, mixed-scope edit, bad token/incarnation | Rejection; subsequent foreground speech completes using the previous valid state. |
| File changed after capture | Both Flite workers install the same captured candidate despite a subsequently invalid file. Concurrent torn-file reads were not separately qualified; existing strict bounded parser tests remain in force. |
| Notification management connection stalled; worker absent | Foreground installs and speaks independently. No all-lane rollback or admission hold. |
| Reply discarded, duplicate retry, stale retry including A→B→A | Query observes actual state; identical content is unchanged; older expected revisions cannot replace newer content. |
| Multipart input, queued legacy text | Multipart request completes once across a reload wake. A held legacy batch postpones installation; Stop releases the boundary and speech continues. |
| Selected profile removed; transient rate and Reset | Removal rejects without losing speech. Unit checks preserve admitted old text and transient rate; Reset retains newly installed punctuation. |
| Read-only management/configuration directories | An actual write fails; readable captured policies still install and speech completes. No persistence success is claimed. |
| 500 reloads with real Flite helpers | Same process identities/start times and descriptor counts. Worker RSS changes by 0 and 44 pages; helper RSS is unchanged. This is a bounded soak observation, not a leak proof. |
| Five Linux worker crashes | Signal only a pidfd-bound fixture child, observe healthy peer speech, complete owned cleanup, start from retained startup and complete a new speech probe. Whole test cycle takes 117–164 ms, including peer speech and replacement cleanup; not a precise first-usable-speech metric. |
| Invalid current configuration during restart | Retained startup restores speech. Live punctuation may revert; current-file parsing is not required for this recovery. |

Lost-requester and malformed/slow management traffic cannot accumulate an unbounded
candidate queue. Thread/socket ownership, one pending slot and existing queue limits
remain necessary. The private header reader still needs a whole-frame deadline;
its per-read timeout can allow header trickling to occupy management longer than
one second. The tests do not establish safety under owner-plus-worker/guard failure,
damaged ownership claims, disk corruption or reboot. Those inherited recovery gaps
must not be disguised as solved by weaker preference consistency.

Emergency speech should use known-good/minimal configuration independently of
preference journals and optional engine discovery, while obtaining defensible
native lifetime authority. If old ownership cannot be resolved safely, use a
verified guard/OS job lifetime or explicit recovery boundary; never delete ownership
records, signal unrelated processes or fabricate cleanup receipts. A functioning
host, usable output and an available engine remain prerequisites. Emergency startup
is a separate design and failure experiment, not implemented here.

## Client, platform and verification coverage

A fresh Emacs 31 batch instance explicitly loaded source from the new Emacsvox
worktree, initialized Speaker and Notify through ordinary owners, installed both
policies without replacing either Emacs process and submitted a real letter command.
The fixture prints and verifies the actual loaded source files. It is not a settings
UI or a production client command.

Locked workspace tests passed: 1,200 tests, two ignored, 38 suite summaries.
The complete optional Piper workspace gate required prepared inputs and a shorter
isolated Cargo target directory after a path-sensitive native build failure.
No source/dependency workaround was used. Relevant locked CLI Clippy with Piper
and all targets passed, as did formatting and Emacs header checks. Failed initial
build/fixture attempts are preserved and excluded from qualified measurements.

Linux was exercised on WSL2. Native Windows staging and interoperability
results are recorded below. macOS,
physical acoustic onset, Stop-to-silence, realistic multi-engine production load,
native Windows Emacs and broad combined-failure recovery remain unqualified.
No available test result should be substituted for those checks.

Full isolated native Windows development staging passed, including deterministic
helpers, payload verification, eSpeak inventory and Flite/RuTTS/TGSpeechBox WAV
smokes. Its build ID is `92d329bf23899a52`. The native test exposed a candidate
capture bug: owned startup canonicalized the executable path, while candidate
capture did not, deriving different spellings of adjacent helper paths. The
identity check correctly rejected the candidate. Capture now uses the owner's
canonicalization boundary; the check was not weakened. The guarded main-only
development rebuild accepted this main-server change on that verified base and
passed its native smoke check, producing `1f0e16310e400af8`.

On the corrected build, a WSL client drove two native Windows Flite workers through
20 successful installations and 20 subsequent speech completions, with ordinary
owned cleanup. Candidate differences were asserted to be only punctuation and
activation identity. Native ownership state required the Windows filesystem;
placing it on WSL's UNC filesystem failed the existing file operations.

WSL could not connect to the private Windows loopback listener in this network
configuration. A temporary PowerShell native TCP bridge verified management
functionality; it is explicitly not a performance solution or a delivered client.
Fresh Windows candidate capture from WSL took 233 ms in the successful run
(206–222 ms in preceding diagnostics). Thus the Linux 7.5 ms full-operation result
must not be generalized to Windows. Use an appropriate local control transport and
resident preparation bound to verified launch context for delivery, then measure
that real client path. Spawning a bridge or capture executable for every reload
would retain avoidable restart-like costs. No Windows latency acceptance or
acoustic qualification is claimed.

## Alternatives and retained guarantees

| Direction | Strongest case | Cost or unresolved dependency | Recommendation |
| --- | --- | --- | --- |
| Finish current session guarantees | Existing preparation, guards, identity and fault tests; coordinated Apply remains valuable for actual asset replacement. Transport and launcher costs may be optimizable. | Additional forwarding, acknowledgement, durable-state and supervision dependencies buy exact cross-stream preferences that are not required here. Prior 8.5/10.5 ms observations are separate historical samples, not this build's measurements. | Preserve as comparator and source of tested protections; do not finish it as a prerequisite. |
| Ordinary owners plus lightweight reload | Demonstrated release-like speech response; inexpensive independent policy change; small added Rust surface and existing clients. | Public status/capability contract, maximum-size admission work, final local transport and combined-failure ownership recovery need delivery work. | Selected next direction. |
| Smaller Rust lifecycle manager, separate speech/reload/Apply | Could make restart and native authority dependable while keeping speech independent of management. | Adds lifecycle/platform code and memory/CPU; needs proof of recovery benefit. Fewer processes alone is not a performance argument. | Reuse guards first; introduce a manager only for a concrete missing lifetime responsibility. |

Retain request coherence, native isolation, resource bounds, prompt cancellation,
cancelled-audio suppression, asset exclusion, local authority and truthful results.
Relax global preference revision, simultaneous installation, cross-worker rollback,
durable exact preference restoration and notification dependence for foreground
progress. Integrity checks belong at capture/execution/asset-use boundaries; this
experiment removes no executable or asset verification and does not justify
silently dropping compatibility checks.

At session revision `7879419`, reconsider ADR 0011's authoritative revision,
coordinated admission and durable all-lane preference completion, and ADR 0013's
central coordination/frozen membership as requirements for ordinary reload.
Extend ADR 0009's frozen host policy/Reset semantics and clarify ADR 0008's immutable
native startup versus live preferences. ADR 0016 remains about expensive managed
activation, not a dependency for punctuation. Preserve ADRs 0007, 0014 and 0015's
asset/lifetime protections and 0001, 0003, 0004 and 0006's isolation, playback,
ownership and compatibility constraints. This older branch's ADR index has not
been rewritten to pretend these later decisions never existed.

The next delivery is the supported per-worker punctuation interface and truthful
status, followed by separate recovery hardening. More settings and rolling engine
replacement should follow evidence, not precede this small useful capability.
