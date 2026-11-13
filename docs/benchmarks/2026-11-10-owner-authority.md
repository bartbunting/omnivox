# Ordinary owner failure boundaries, 2026-11-10

## Result

Keep independent preference reload and minimal emergency startup. The next
necessary recovery improvement is a native cleanup authority whose communication
and lifetime are independent of speech forwarding. Optimizing preference
transactions does not fix the demonstrated failures. Reuse the session guard's
native ownership and transfer protections selectively; do not import global
preference Commit/Abort or managed Apply as a prerequisite.

The [preceding emergency slice](2026-11-10-emergency-speech.md) is committed at
Omnivox `2ad8338` and Emacsvox `25cc0e24b`. It restores speech after corrupt
preferences, damaged management state and failed receipt writes while its original
native owner remains reachable. This follow-up deliberately tests that last
assumption. It changes no runtime or installed speech.

## Method and observations

The [fixture and raw evidence](data/2026-11-10-owner-authority/README.md) use the
same Linux payload, SHA-256
`ef4310a952c25ad4bba4bf97f6d87e7e4667379932dfee1d7527314f27ba76bf`.
Each case starts a fresh ordinary eSpeak owner with private state and null output.
The fixture retains pidfds before faults, checks the actual child relationship,
and acts as a dedicated subreaper solely for eventual test cleanup. It does not
give those handles to the product or manufacture receipts. No saved PID or other
client's process is signalled. A separate healthy owner speaks a character after
each fault is injected.

The final run observes each fault for at least 6.25 seconds, then queries the
existing exact `retirement-status` service before test cleanup. This is a bounded
functional counterexample, not a latency distribution:

| Fault | Native worker at observation | Product cleanup evidence | Healthy peer |
| --- | --- | --- | --- |
| Worker paused; owner receives a 64 KiB ordinary queued-text record followed by retirement | Still alive; owner main thread blocked in `anon_pipe_write` | No reply; exact durable status false | Character completed; source 1.408 ms |
| Client deliberately stops reading worker output; capabilities fill the pipe; retirement follows | Retired | Exact durable status true; reply could not be consumed while output was held | Character completed; source 2.243 ms |
| Owner killed with an otherwise responsive worker | Worker exited | Exact durable status false; lost owner cannot author the receipt | Character completed; source 1.914 ms |
| Owner killed after pausing its worker | Worker still alive and paused | Exact durable status false | Character completed; source 1.416 ms |

The output case is different from the other three: durable proof exists. Draining
the held output during fixture cleanup reveals the real `native_retired` reply.
Do not infer missing native cleanup merely from a missing client reply. Conversely,
the fixture's observation that a worker exited after owner death is not the
product's authorized native-tree/I/O proof. The paused-worker case shows why exit
or EOF alone cannot safely replace that proof.

The first complete run observed the same four outcomes without the service query.
Its EOF observation loop could spin after owner exit; the final fixture sleeps
there and keeps that preliminary evidence separate. An intervening run failed
because the fixture mistakenly prefixed a service request with the owner marker;
the corrected run uses the documented raw JSON service input. All logs and
fixture revisions are retained. Test cleanup killed only retained exact handles;
it is explicitly not a successful product recovery.

## Consequences for the next slice

A nonblocking writer or dedicated thread can remove the immediate input stall,
but cannot provide an authority after the owner process dies. A faster poll cannot
run while the owner is blocked in a pipe write. A separate output status path can
recover an already durable result but does not solve failed persistence or lost
native authority. These distinctions argue for one measured lifecycle extraction,
not another series of preference consistency mechanisms.

The retained session implementation at `7879419` already separates native cleanup
from forwarding. Its Unix guard retains unreaped group leaders and its Windows
guard retains actual process/job handles, restricts inheritance, gates launch and
accounts separately for tree and I/O retirement. The accepted
[0014](data/2026-11-10-lightweight-reload/0014-unix-session-cleanup-guard.txt) and
[0015](data/2026-11-10-lightweight-reload/0015-windows-session-cleanup-guard.txt)
remain relevant protections; [0018](../adr/0018-minimal-emergency-speech.md)
permits their reuse without a management-history dependency for minimal speech.
This review inspected the retained source and historical qualification; it did
not rerun those historical suites or establish a new integrated guard result.

Extract only creation before START, retained native identity, independent lifetime
and control, bounded transfer and separate tree/I/O completion. Keep old claims
until existing durable proof permits their release. Minimal recovery may use
an actual surviving authority's transient proof when storage fails. The guard
should not select preferences or participate in ordinary reload. Speech should
retain its existing forwarding depth, and an ordinary management-service failure
should not terminate healthy workers.

Before implementation acceptance, the smallest candidate must pass this same
input/output fault fixture, owner loss with an unresponsive worker, worker death
before owner loss with a surviving helper, and interrupted launch/handle transfer.
Then kill the guard itself: missing authority must remain explicitly blocked,
with no new worker, fake receipt or released claim. Exercise failed receipt writes
separately from native cleanup. Measure startup cost, resource bounds and matched
steady-state speech against `v1.18.0` before expanding managed activation.

Linux process groups and Windows kill-on-close jobs are not interchangeable.
Windows may terminate a worker tree after owner death while still losing the
receipt writer; a retained observer handle can change job lifetime. WSL transport
needs native Windows qualification. macOS needs its own native authority and I/O
checks. The Linux results above make no Windows, macOS, acoustic or reboot claim.

This establishes the next delivery requirement, not completion of the overall
availability goal. The [plan](../plans/lightweight-preference-reload.md#emergency-recovery-under-adr-0018)
tracks that remaining implementation and qualification.
