# Independent native guard: first Linux owner/client integration

The actual Rust owner and paired Emacs client now support an opt-in independent
cleanup guard under [ADR 0018](../adr/0018-minimal-emergency-speech.md). Local speech
uses stdio; guard control uses Linux abstract Unix-domain sockets through a small
stdio bridge. There is no TCP listener or additional speech forwarder. The
[private contract](../reference/engine-startup-snapshot.md#opt-in-independent-linux-cleanup-guard)
records exact identities, bounds and native ownership.

Continue this direction as an experiment. The Linux results establish useful
owner-loss recovery. Small speech-latency differences still need attribution,
and the extra startup cost is measurable. Keep it disabled by default
until integrated partial-transfer/status-backpressure checks and native
Windows/WSL qualification are complete. This is not a deployment, release,
acoustic result or platform-wide acceptance.

## Implementation and proof

The owner prepares one guard and retains a private inherited control connection.
The client receives exact guard/member identity and attaches independently before
native creation. The guard spawns the ordinary worker behind the existing START
snapshot gate, then drops its copies of the speech endpoints. The owner forwards
speech directly to that worker. Every forwarding endpoint and startup writer
retains a completion lease until its actual I/O closes.

The guard observes the actual owner pidfd, retains the existing Linux subreaper
and private process-group protections, and reports tree retirement independently
of I/O retirement. An explicit emergency request first retires the tree, then
permits termination through the retained owner handle, then requires both facts.
It writes no management records. Ordinary owner receipts, managed-asset claims,
helper isolation, cancellation and the ordinary preference path remain distinct.

A native stdio bridge was selected because the tested Emacs abstract-socket route
failed with an invalid-argument response. A private filesystem Unix socket worked,
but would add a writable-directory dependency to emergency control. Both probes
are retained. This is evidence about the tested Emacs invocation, not a claim
that every Emacs implementation lacks abstract-socket support. Windows named
pipes remain the intended next native transport, not implemented by this slice.

## Native and paired-client results

The final native run uses real Flite helpers and null output. Fixture cleanup uses
handles to its own newly created processes; it supplies no product cleanup proof.
Descriptor observations confirm the guard retains neither worker speech endpoint
and the worker inherits no owner pidfd. Damaged host/configuration bytes remain
unchanged through cleanup.

| Injected fault | Final native observation |
| --- | --- |
| Owner killed | Tree and I/O proof in 39.9 ms |
| Owner paused | Tree and I/O proof in 39.1 ms |
| Worker paused; owner blocked writing speech | Actual pipe-write wait observed; proof in 40.5 ms |
| Owner blocked writing its stdout | Actual pipe-write wait observed; proof in 41.6 ms |
| Worker killed; paused helper survives; owner paused | Surviving helper retired; proof in 38.3 ms |
| Guard killed | Replacement remains blocked; no fabricated proof |

These are individual fault trials, not a latency distribution. A preceding
six-case run also passed. Earlier runs and failed fixture assumptions remain in
the raw archive.

Fresh Emacs 31.0.90 checks load the changed source explicitly, then separately
verify actual `.elc` paths for the compiled client. The compiled client restores
minimal eSpeak after paused-owner and killed-owner faults, repeats recovery, and
retries an injected lost guard reply. A real Flite notification lane remains
usable, with the same process identity and unchanged management/asset bytes.
Malformed preferences and host history are present; the sessions directory denies
writes. First restoration after a lost reply is rejected safely; a second explicit
attempt queries retained authority and succeeds without creating duplicate speech.

The final compiled run reports operation-ready times of 147.2, 137.7 and
153.5 ms for paused, killed and repeated recovery. The lost-reply rejection plus
explicit retry takes 238.6 ms. First requested source events follow at 164.7,
153.8, 170.5 and 254.1 ms from operation start. These include preparation and client
initialization; they are neither acoustic blackout nor time to first PCM. Earlier source and compiled runs remain in the archive.

The paired tests exposed a real bookkeeping bug: bounded cleanup of a completed
control connection initially discarded a received native proof before emergency
recovery used it. The client now retains that monotonic observation without
turning it into a durable receipt or permitting asset removal. A regression covers
this sequence. Other regressions reject missing tree/I/O proof, unavailable
authority, stale/foreign responses and oversized incomplete replies.

## Matched speech and startup measurements

The final comparison uses the same candidate executable, Python client, engines,
null output and workload for direct and guarded owners. Two blocks per engine
reverse direct/guarded order. Each has two live lanes, 50 capability exchanges,
20 foreground character requests during reload, 20 concurrent notification
characters and 20 completed preference reloads. Source time is submission to
`utterance_started`; protocol time is a separate capability round trip.
The final fixture waits until the reload command has been transmitted before
submitting the foreground character. All 40 requests per engine/variant start
inside their recorded reload operation interval. The initial concurrent-task
fixture only ensured overlapping execution and is retained separately. Raw
samples and request/operation intervals remain available.

| Engine and owner | Source median, 40 requests | Observed source range | Protocol median, 100 requests | Two-lane ready median, 2 starts |
| --- | ---: | ---: | ---: | ---: |
| eSpeak, previous direct build | 2.716 ms | 1.557–152.152 ms | 0.899 ms | 182.6 ms |
| eSpeak, candidate direct | 3.018 ms | 1.611–7.110 ms | 0.987 ms | 217.8 ms |
| eSpeak, candidate guarded | 3.257 ms | 1.500–7.235 ms | 0.991 ms | 294.0 ms |
| Flite, previous direct build | 4.597 ms | 2.756–9.964 ms | 0.915 ms | 166.8 ms |
| Flite, candidate direct | 5.414 ms | 3.318–12.009 ms | 1.200 ms | 236.6 ms |
| Flite, candidate guarded | 6.073 ms | 3.147–15.920 ms | 1.111 ms | 315.8 ms |

The previous build is the retained deferred-start candidate, not published
1.18.0. It ran in separate earlier blocks, so its comparison has an order and
host-load limitation. Published-release comparisons remain in the existing
[evidence index](README.md); these numbers do not replace them. The 152 ms
previous-build source outlier is retained, as is a similar outlier in the first
socket-based draft. Neither has been causally attributed. Small sample counts
cannot establish tail behaviour.

The first guarded draft also used Unix socket pairs for worker stdio. The
stricter probe showed source medians about 0.66 ms higher for eSpeak and 0.55 ms
higher for Flite than direct owners in that build. The final implementation
retains ordinary anonymous speech pipes. Its guarded medians are about 0.24 ms
and 0.66 ms higher than its matched direct owners. This does not prove sockets
caused the difference or pipes solved it: host load and order vary, and the
remaining small difference needs a larger controlled comparison. No performance
breakthrough or release-latency equivalence is claimed.

Two-lane startup adds about 76–79 ms relative to candidate direct startup in the
final run. That includes extra executable launches, verification and lifecycle
exchanges; their individual contributions have not been isolated. The 5 ms
lifecycle polling interval is not a speech poll. Preparing the replacement guard
and bridge before old retirement can move preparation outside a healthy restart
blackout; it cannot hide preparation after speech has already failed.

The next bounded performance experiment should compare readiness-based lifecycle
waiting with this polling implementation, retaining the same speech pipes and
native failure matrix. Measure idle wakeups, ordinary speech during reload and
restart separately; do not assume an event mechanism resolves the remaining
latency difference. Stop before platform/default enablement if it loses proof,
introduces speech forwarding or fails the matched responsiveness criteria.

## Verification, provenance and limits

The isolated Omnivox worktree starts at `9316c75`; the paired Emacsvox worktree
starts at `087270479`. The final complete `make build` payload executable is
`24feec5c1ef5c1b56df444063cec67471b49e9230759a079fc36436d6d340227`.
The previous frozen candidate remains
`b06f827455a8b71bf2330f1d00f07088f29e4598a91b4a524f2f0adfb1aac8c4`.
Source patches, payload member hashes, fixtures, native/client traces and check
logs are in the [evidence pack](data/2026-11-10-guard-integration/README.md).

Locked workspace tests and CLI/TTS Piper Clippy pass with Rust 1.97.1. Formatting,
paired documentation checks, Lisp header/build checks and fresh compiled recovery
regressions are recorded in the pack. The core Emacs suite has 1,073 passing tests
and 18 explicit skips. Initial fixture failures include an empty environment
variable selecting stale byte-code, a kernel wait-name variation, and a missing
new-module build inventory entry. All were corrected and rerun; failed logs remain.

Remaining work is explicit: integrated partial bootstrap/START and held status
output qualification; broader cancellation/sustained-speech measurements through
the guarded path; real Windows/WSL named-pipe/handle integration; native macOS;
expensive-engine and acoustic/listening checks. Earlier standalone guard coverage
does not establish these integrated cases. Combined owner/guard loss and reboot
reconciliation are still unqualified. No installed runtime or maintenance branch
was changed, and no accepted decision was rewritten.
