# Independent native guard extraction, 2026-11-10

## Result and scope

A small independent Rust guard can resolve the demonstrated Linux owner failures
without forwarding speech or involving preference commits, managed Apply or
writable history. Retain this direction under
[ADR 0018](../adr/0018-minimal-emergency-speech.md). The
[fixtures and raw observations](data/2026-11-10-independent-guard/README.md)
contain an isolated extraction experiment, not an enabled Omnivox/client feature.
The current paired client still has the
[ordinary owner limits](2026-11-10-owner-authority.md).

Three runs of the final seven-case matrix pass with real Flite workers/helpers,
an independent healthy notification owner and bundled eSpeak replacement. The
guard is Rust; the fixture forwarding process is Python. No native library is
moved into the guard. Production integration, Windows handle transfer and native
macOS qualification remain outstanding. This is a decisive test of the ownership
separation, not proof that integration is complete or latency-neutral.

## Mechanism

The standalone probe uses the repository's exact Linux native tree implementation
and the session guard's termination order: retain the unreaped group leader,
signal its private group once, reap the actual child, then verify group retirement.
It holds one native attempt and never selects preferences or writes management
records. Creation precedes native START. A partial START cannot initialize engines.

Speech pipes run directly between the forwarding owner and native worker, with
the same one forwarding hop as ordinary ownership. Independent anonymous pipes
carry lifecycle requests/status and the owner's lifetime/I/O leases. The fixture
precreates and transfers these descriptors before launch. Actual `/proc` checks
verify that the guard retains no speech endpoint and that neither worker nor
helper inherits the lifetime descriptors. The production cross-process transfer
and client negotiation protocol are deliberately not supplied by this fixture.

The native loop checks lifetime and retirement every 5 ms. It reads at most
128 control bytes per iteration and retains one bounded status reply, including
partial-write state. Its status output is nonblocking. No speech write, reply
consumer or disk write can block that loop. The interval affects lifecycle
observations only; ordinary speech never traverses this poll.

Tree and I/O retirement remain separate. The forwarding fixture retains its
completion lease until both I/O owners finish; process death closes the lease and
all process-owned endpoints. A live native guard's observation permits the test
client to start a separate minimal owner. It does not release old persisted claims
or manufacture durable receipts. Losing the guard leaves no such authority.

## Final failure matrix

| Fault | Observed native result | Replacement condition and peer |
| --- | --- | --- |
| Paused worker; forwarding blocked writing 64 KiB | Independent retirement kills worker and real helper | Both tree and I/O proof precede eSpeak replacement; notification character completes |
| Client holds speech output; both forwarding threads blocked | Tree retires while I/O remains explicitly false | Exact retained forwarder is terminated; I/O then retires; replacement and notification complete |
| Client fills and holds guard status output | Native cleanup continues despite actual nonblocking write stalls | Client must drain old replies and obtain current complete proof before replacement; notification completes |
| Forwarding owner dies with worker paused | Lifetime loss retires worker/helper | Actual tree/I/O proof precedes replacement; notification completes |
| Worker dies first, leaving a paused helper; owner then dies | Unreaped worker identity still permits group termination | Helper retires; complete proof precedes replacement; notification completes |
| Owner dies during incomplete native START | Gated worker retires without engine startup | Complete proof precedes replacement; notification completes |
| Guard dies while worker is paused | No live proof; worker remains | Test client blocks replacement; notification continues; retained fixture handles perform later cleanup only |

The guard-loss case's final pidfd cleanup is not product recovery evidence.
Fault injection and fixture cleanup use actual retained handles, never saved PIDs
or unrelated processes. Private configuration/history is damaged and its receipt
directory unwritable throughout the matrix. All old management bytes stay equal.

For the five ordinary successful cases, final-run observations receive complete
tree/I/O proof about 9–25 ms after the fault or retirement request. These are three
functional samples per case, not latency guarantees or tail estimates. In the
deliberately flooded status case, native processes exit while replies are still
held; receiving current proof takes about 686–695 ms because the fixture then
drains stale buffered responses. This distinguishes cleanup from delivery of its
evidence. A production client should allow only bounded outstanding requests and
correlate the exact incarnation and operation; it must not infer proof from the
observed process exit or timeout.

The earlier probe failures are retained. One fixture initially mistook completion
of its outer write for absence of a blocked forwarding thread; actual thread wait
states fixed that check. Another expected a nonblocking pipe to fill every byte,
overlooking atomic small writes and unused padding. The final guard counts actual
`WouldBlock` events, and all three final status-flood runs observe them.

## Integration acceptance

Reuse the native tree/gate, lifetime separation and bounded polling concepts.
Preserve the session implementation's stronger descriptor/handle transfer and
identity protections rather than promoting this fixture protocol. The smallest
next product increment is one negotiated guarded worker with a client-held
lifecycle connection independent of its forwarding owner. Retain its guard/member
identity before any native attempt, account for partial transfer, and keep exact
tree and I/O proof available after owner loss. Then add independent membership for
the other lane without a preference transaction or new speech hop.

Windows requires actual retained process/job handles, explicit inheritance and
partial-duplication cleanup; this Linux lease experiment cannot substitute for
those checks. macOS requires its own group and I/O qualification. Guard loss,
uncertain transfer and combined failures must remain explicitly unavailable.
Reboot reconciliation is a separate boundary. Before enabling integration, repeat
this matrix through the compiled paired client, retain old failed-attempt handles,
measure matched release speech and recovery, and qualify Windows/WSL separately.

The probe compiles with pinned Rust 1.97.1 and warnings denied. Its source format,
unchanged platform-source hash, archive readback and documentation links are
checked. It adds no production runtime source, dependency, service or installed
payload. The preceding product slice's workspace, Clippy and paired client checks
remain recorded in the [prepared-start report](2026-11-10-deferred-start.md).
