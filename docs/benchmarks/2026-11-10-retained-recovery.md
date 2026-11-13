# Ordinary retained-startup recovery

This delivery fixes a client availability failure independently of preference
reload. The native payload remains the one qualified in the
[host-preference report](2026-11-10-host-preferences.md); this slice changes the
paired Emacsvox client. No installed speech runtime or live Emacs was changed.

## Failure and repair

A local native worker can exit while its owner stays alive to answer management
commands. After verified native cleanup, that owner sends a `retired` receipt.
The old client checked only whether the owner process was alive, so it could
continue sending speech to an empty owner indefinitely. An isolated Linux crash
reproduced this failure before the fix.

Speech and character delivery now recognise the retirement receipt. The first
subsequent request recovers from the retained startup, then waits boundedly for
that worker's initial voice registration before sending speech. Startup callbacks
register only their own worker, avoiding unnecessary registration changes on the
healthy peer. Notification fallback keeps the real foreground process binding;
a raced notification failure cannot create an unreachable replacement owner.

`M-x tts-recover` exposes the retained route explicitly. It obtains actual native
retirement before replacement and keeps notification speech running. Both this
command and automatic recovery refuse to replace a dead local owner without
retirement evidence. The process sentinel retains that dead handle, so a cleared
global cannot bypass the check; fresh restart checks it too. `tts-restart` remains the fresh-settings route: preparation
failure preserves the existing foreground worker. This slice does not qualify
fresh restart after combined owner/worker failure.

Recovery may restore older host preferences. Reload them again after repairing
the saved file. No ownership record is deleted, no saved PID supplies signalling
authority, and asset claims remain governed by their existing contracts.

## Native observations

Tests use private configuration and voice roots, real synthesis and null output.
Linux crash injection verifies the isolated launcher's executable/ancestry and
holds pidfds before signalling its exact worker. It never signals a saved PID or
an unrelated process. Windows uses the WSL client and the isolated native runtime;
Windows worker-crash injection is not claimed.

The source fixture verifies actual engine IDs, preserves the notification owner
and completes notification speech after recovery. A malformed current file makes
fresh restart fail without replacing working speech; retained recovery still
completes. The first post-crash request itself must complete, rather than relying
on an unobserved warm-up utterance.

Initial functional observations, not a matched timing comparison:

| Route | Engine | Client recovery to completed speech |
| --- | --- | --- |
| Linux explicit retained recovery, source-final | Flite | 162.478 ms |
| Linux explicit retained recovery, piper-source-1 | Piper/Lessac medium | 772.573 ms |
| WSL to native Windows retained recovery, windows-source-1 | Flite | 835.814 ms |

After the owner had reported retirement, the first post-crash request reached its
source-start event in 128.566 ms for Linux Flite (`source-final`) and 711.857 ms
for Piper (`piper-source-1`). These are individual observations. Some runs overlap
builds or tests; they establish functionality, not a distribution, blackout bound,
acoustic onset or a speedup over the release. Explicit recovery timing includes
retirement, startup, client registration and the test utterance. First-source
measurement starts at the request; crash detection before that is excluded.

## Failure boundaries

| Failure | Available speech and recovery | Remaining dependency |
| --- | --- | --- |
| Invalid saved configuration or rejected reload | Existing workers keep their policy. Retained recovery bypasses the current file. | Valid retained startup, engine/output and writable owner records. |
| Unavailable preferred engine | Existing routing may choose an available fallback; the earlier resident report exercised missing Flite. | A usable fallback and correct native routing. Not newly measured here. |
| Native worker crash with surviving owner | Peer continues; first main request uses confirmed retirement and retained startup. | Owner completes native cleanup and reports it. |
| Notification worker retirement | Foreground serves notification fallback; foreground recovery is retained globally. | Available foreground, or its ordinary recovery prerequisites. |
| Lost retirement reply | Peer continues; explicit owner retirement exits after replying, so this connection cannot supply a retry. Replacement remains blocked. | Reconcile the existing durable retirement receipt; this is the next ordinary recovery increment. |
| Owner loss or missing native cleanup evidence | A healthy peer remains usable; this route refuses unproven replacement. | Native authority or a separately qualified OS recovery boundary. |
| Damaged retained history, stale claims or unwritable management storage | Already working speech and preference reload do not need new lifecycle transactions. Retained restart can still fail. | Management repair; emergency startup is not implemented. |
| Reboot | Old native processes cannot continue, but damaged persisted records can still obstruct ordinary startup. | A separately specified and qualified emergency boundary; not tested here. |

The ordinary path has no session supervisor/guard. Its combined-owner-failure
coverage cannot be inferred from the session experiment. Native ownership,
cancellation, bounded queues, managed asset exclusion and helper isolation remain
necessary even though preferences now converge independently.

## Verification and evidence

Fresh source and compiled Linux Flite and Piper checks passed, as did the fresh
compiled WSL-to-native-Windows Flite check. Actual loaded Lisp paths are asserted.
The final fixture also proves the peer's acknowledged registration is unchanged.
Lost-reply injection passed on Linux and native Windows by retaining the uncertain
dead owner, refusing a replacement and completing notification speech. That is a
safety result with an explicit availability gap, not successful main recovery.
The native owner already persists a retirement receipt before replying; a small
native verification interface can reconcile it under ADR 0007 without a new
emergency policy or a fabricated receipt.

Nine focused recovery regressions passed. The final client core suite passed
1,055 tests with 18 conditional skips; four earlier failures were fake-adapter
fixtures lacking a process-command stub and are retained with the corrected run.
Byte-code, headers, generated manuals, client documentation and paired Omnivox
documentation gates passed. Their logs are retained in the pack. Full byte-code
rebuild passed, followed by incremental rebuilds after the deadline and sentinel
fixes. The targeted retirement wait is six seconds, allowing the native owner's
five-second cleanup deadline without inheriting the 45-second asset-management
wait. Initial voice-registration readiness remains bounded at two seconds.

The
[evidence pack](data/2026-11-10-retained-recovery/README.md) retains the source
patch, fixtures, original failed attempts, check logs and payload identities.

The initial fixture attempts exposed an unsettled registration and inherited
logger children; failures occurred before unsafe signalling. Baseline-5 reproduced
the empty-owner bug. These attempts are retained rather than silently replaced.
Native macOS, physical output and Stop-to-silence remain unqualified. The native
payload's earlier Rust tests, Clippy and native Windows staging remain applicable;
this Lisp-only slice does not require another native rebuild.

## Next boundary

Keep the direct speech path and independent reload. Ordinary recovery is a useful
increment, but it does not satisfy recovery through damaged management storage or
loss of every native authority. The
[emergency proposal](../plans/lightweight-preference-reload.md#next-recovery-experiment-and-proposed-emergency-boundary)
needs its separate ADR approval. First expose read-only verification of the already-written native retirement
receipt, with wrong identity, missing, partial and changed-record rejection.
Then the separately approved emergency experiment can exercise minimal owned
speech without management reads/writes after verified retirement, while retaining
all old claims. Do not import global preference transactions to solve that problem.
