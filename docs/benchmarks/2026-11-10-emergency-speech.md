# Minimal owned emergency speech, 2026-11-10

## Result and scope

Continue the lightweight design and selectively reuse native lifetime protections.
The explicit minimal path restores bundled eSpeak when configuration or management
storage is unusable, without adding work to ordinary request admission. A surviving
owner can prove actual native cleanup despite a failed receipt write; retained
claims and damaged history remain intact. This closes a storage dependency in
speech recovery. It does not establish recovery after losing every native authority
or while the owner's shared forwarding transport is blocked.

This implements the separately accepted [ADR 0018](../adr/0018-minimal-emergency-speech.md).
It extends the ordinary recovery and lost-reply work at Omnivox `26843cd` and paired
Emacsvox `7369d3e83`. The exact [startup/retirement contract](../reference/engine-startup-snapshot.md#explicit-minimal-emergency-startup)
owns wire fields and commands; the [plan](../plans/lightweight-preference-reload.md)
owns remaining work. All changes and runtimes remain isolated in the lightweight
worktrees. Release `v1.18.0`, maintenance main, the session experiment, installed
speech and previous evidence are unchanged. No deployment or release is implied.

## What changed

`--emergency-owner` constructs only the bundled eSpeak startup in memory, with
built-in host policy and an explicit output backend. It neither opens a management
host nor reads preferences, active pointers, retained startup files or optional
engine registrations. It uses the ordinary owned START gate, bounded queues,
generation cancellation and playback. A Windows bundle may retain its existing
separate pinned eSpeak data directory; the launcher resolves that installation
reference while bypassing preference conversion and disk logging.

`retire-native` reports actual tree and pipe retirement independently of durable
receipt success. The owner remains queryable. A false `retirement_recorded` never
unlocks managed assets or repairs the failed receipt. The paired
`tts-emergency-start` preserves notification speech, uses the original launcher
context and can reconcile a lost reply from the same surviving authority.
Repeated retained recovery keeps minimal mode; deliberate restart captures
ordinary settings again. Managed Apply rejects a minimal worker. A failed new
initialization retains the attempted owner until cleanup is confirmed, so retry
cannot forget a possibly surviving worker. A live client Apply must finish or
cancel before emergency replacement; damaged persisted history is not that gate.

## Native and client observations

The [raw pack](data/2026-11-10-emergency-speech/README.md) contains fixture sources,
raw responses, source-event timestamps, failures and build/check provenance.
Native runs use real Flite before the failure and real bundled eSpeak afterward.
The client also covers recovery from an actual Piper Lessac worker. All use null
output. A source event follows actual PCM entering the source playback path, but
does not establish first acoustic output, Stop-to-silence or perceived quality.

Eight independent minimal starts per platform produced these client-observed
startup-to-first-source timings, including capability and probe registration:

| Native target | Minimum | Median | Maximum |
| --- | ---: | ---: | ---: |
| Linux | 40.425 ms | 41.042 ms | 42.479 ms |
| Windows through WSL | 315.061 ms | 329.846 ms | 355.338 ms |

These are small functional samples, not tail-latency guarantees. The complete
client operation also includes capability probing, old-worker retirement and
Emacs readiness acknowledgements. Source-loaded Emacs observations were:

| Original engine/target | Emergency command | Concurrent notification submitted / first source / completed |
| --- | ---: | ---: |
| Piper, Linux | 129.405 ms | 10.188 / 113.860 / 114.218 ms |
| Flite, Windows through WSL | 660.966 ms | 10.164 / 33.097 / 33.581 ms |

These are individual observations from retained runs. Times in the last column
are relative to command entry. The notification retains its original owner and
registration. Operation duration includes time when the old worker can still
speak; it is not a measured blackout interval. Actual acoustic blackout remains
unmeasured. Raw runs also retain roughly 150 ms source-event outliers, already
seen in previous release comparisons; this slice does not attribute their cause.

Final fresh compiled-client checks explicitly verified the loaded `.elc` paths:

| Original engine/target | Emergency command | Notification submit / first source / completed | Retirement request to first new source |
| --- | ---: | ---: | ---: |
| Flite, Linux | 84.119 ms | 10.134 / 27.202 / 27.323 ms | 93.544 ms |
| Piper, Linux | 75.496 ms | 10.192 / 71.550 / 71.630 ms | 84.495 ms |
| Flite, Windows through WSL | 556.080 ms | 10.227 / 27.148 / 27.300 ms | 414.173 ms |

The last column bounds the source-level recovery gap: it starts before native
retirement and ends at the first explicitly requested new source event, including
client readiness and submission. It is not an acoustic blackout measurement or
a distribution. Linux runs additionally killed the exact emergency worker through
retained pidfds and verified that the next character restored minimal speech.
All three checks preserved the healthy notification owner, its registration and
every retained management-file byte through repeated minimal restarts.

## Matched ordinary-path regression

Four rotated blocks compare the same real eSpeak/null-output fixture against
release `246e3b7`, the preceding receipt build and this candidate. Each cell has
1,024 successful requests; all 8,192 requests completed. Values are median /
nearest-rank empirical p95 milliseconds from request to source event:

| Workload | Release | Previous receipt build | Candidate idle | Candidate with reload |
| --- | ---: | ---: | ---: | ---: |
| Short ordered speech | 1.709 / 2.436 | 1.634 / 2.428 | 1.687 / 2.401 | 1.861 / 5.022 |
| Character speech | 1.729 / 2.426 | 1.782 / 2.425 | 1.788 / 2.533 | 1.879 / 5.256 |

The 512 requests per workload whose intervals actually overlap reload have
median / p95 3.129 / 5.655 ms for ordered speech and 3.568 / 5.789 ms for
characters. Reload-operation median / p95 is 8.434 / 12.110 ms and
8.973 / 12.834 ms respectively, with 1,024 successful lane operations per
workload. An impaired lane, Stop and replacement retain their preceding
host-preference qualification; this matrix is an ordinary-path regression check.

Every cell retains rare events above 100 ms: 1–10 per cell, with maxima
152–156 ms, including release. These events are not discarded or attributed to
the new path. Early matrix blocks overlapped an Emacs byte-code build; rotation,
timestamps and load averages are retained, but the host was not exclusive.
The data supports continued development under the proposed screening thresholds,
not an acoustic tail-latency guarantee. The pack contains raw rows and the exact
summary script rather than only rounded aggregates.

## Failure coverage and limits

| Failure or condition | Speech available and recovery | Evidence/limit |
| --- | --- | --- |
| Malformed saved preference file | Existing foreground and notification speech continue; reload rejects; explicit minimal startup succeeds after old cleanup. | Native Linux/Windows and fresh Emacs. |
| Damaged `host.json` | Existing owners continue; emergency owner does not open the damaged host. | Damaged bytes are unchanged. |
| Receipt write fails | Live owner retires actual native tree/I/O and reports transient proof. Minimal speech starts; claims remain retained. | Linux denies session-directory writes. Windows obstructs each receipt path with a directory: a write failure, not simulated global disk-full or power loss. |
| Lost native-retirement reply | Client uses the same original live authority or its received exact observation; no new owner infers cleanup. | Fresh client injection after an actual native result. |
| No usable management/configuration directory | Minimal startup succeeds with those paths naming a regular file. | Native fixture verifies the original file is untouched. |
| Emergency worker crashes | Surviving owner confirms cleanup; the next character restores minimal mode. | Linux pidfd-bound crash injection; native Windows crash order not established by this fixture. |
| Missing or mismatched cleanup proof | No replacement is admitted. Healthy peer remains available. | Client regression tests and preceding retained-receipt evidence. |
| Owner/guard lost, or forwarding I/O blocked | No newly proven recovery route in this slice. Existing valid durable evidence can still serve ordinary recovery. | Never infer safety from PID, exit, EOF or free lock. Independent lifecycle authority and reboot reconciliation remain unqualified. |
| Bundled eSpeak/data or selected output unusable | Minimal mode cannot promise speech. Existing ordinary recovery may still work; otherwise repair the core payload/output. | Available optional engines alone do not qualify this deliberately minimal route. |
| Machine reboot | Old processes cannot survive a verified host reboot, but no automated history reconciliation is implemented. | Preserve old evidence and asset exclusions; this report does not invent a boot proof. |

The changed path retains helper isolation, native ownership before initialization,
bounded input and synthesis queues, per-request policy coherence and suppression
of cancelled audio. Ordinary preference reload remains independent of managed
Apply and durable writes. Emergency mode creates no managed-asset load and has
no permission to release older pins. The selected executable/data bundle remains
a required trusted installation input.

## Verification and remaining acceptance

Locked Rust workspace tests pass: 1,213 passed, two ignored across 38 result
suites. Relevant CLI/TTS Clippy with Piper and the pinned formatting check pass.
The final source client core gate passes 1,063 tests with 18 explicit skips.
Seventeen focused recovery tests pass. The first byte-code build accidentally selected
system Emacs 30.2 and failed on an unrelated installed chess dependency; its log
is retained. The full rebuild and subsequent incremental check passed with
explicit Emacs 31.0.90. Full Windows
development staging and native helper/runtime checks pass in the private Temp
runtime. Fresh compiled-client checks pass for Linux Flite/Piper and native
Windows through the WSL client. Documentation and evidence checks are retained
in the pack. Source patches and payload manifests distinguish the native build
inputs from subsequent client-only hardening.

Native macOS, native Windows Emacs and acoustic/listening acceptance are still
unavailable here. No new numerical production guarantee is claimed. Proposed
screening targets remain release-like ordinary speech, no foreground hold for
secondary preference failure, and minimal native first source below 100 ms on
this Linux setup and 500 ms on this WSL-to-Windows setup. These new targets are
proposals tied to the tested hardware and null output; representative real-output
measurements must establish user-facing targets.

The next discriminating experiment is a bounded owner-input/output stall and
owner-loss test, using retained native authority and no installed speech. Compare
an independent lifecycle channel/guard with the current ordinary owner. Reuse
the session guards if they close that measured failure without adding a speech
forwarding hop or a dependency on durable preference transactions. Do not complete
managed Apply infrastructure merely to obtain that evidence.
