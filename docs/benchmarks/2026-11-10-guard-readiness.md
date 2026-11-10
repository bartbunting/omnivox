# Native guard readiness and interrupted recovery

Keep the independent guard direction. OS readiness removes idle wakeups and
shortens control exchanges without inserting speech forwarding. The actual Linux
owner now passes interrupted bootstrap/START, held status output and disconnected
client checks. Windows/WSL and macOS qualification remain separate. This opt-in
slice does not change deployment, defaults or the release baseline.

This follows the [first integration](2026-11-10-guard-integration.md).
[Provenance](data/2026-11-10-guard-readiness/provenance.json),
[raw observations and fixtures](data/2026-11-10-guard-readiness/raw.tar.gz),
[checksums](data/2026-11-10-guard-readiness/SHA256SUMS), and exact source patches
are retained. Omnivox source is `7c508e3`; the event executable SHA-256 is
`86346921b4c08af7cb224b7a3ddf50909dfbf70f0ed4e90b363c505e8709d7c8`.
The preserved polling executable is from the preceding integration. Both use
anonymous speech pipes, abstract Unix-domain guard control and no TCP.

## Measurements

Four alternating direct/guarded blocks per engine contain 80 character requests
submitted after the actual reload command, concurrent notification characters,
and 50 capability exchanges. Each engine/variant therefore has 320 source samples,
320 successful reloads and 200 capability exchanges. Builds were not running
in these blocks. Polling and event binaries ran in separate sequential sets;
host/order variation still limits causal attribution. These are source events,
not first PCM, acoustic onset or a fresh release-1.18 comparison.

| Variant | Engine | Direct source median | Guarded source median | Direct two-lane ready median | Guarded two-lane ready median |
| --- | --- | ---: | ---: | ---: | ---: |
| Polling | eSpeak | 3.236 ms | 3.288 ms | 214.7 ms | 283.1 ms |
| Readiness | eSpeak | 2.901 ms | 3.029 ms | 203.6 ms | 249.1 ms |
| Polling | Flite | 5.274 ms | 5.416 ms | 176.2 ms | 272.8 ms |
| Readiness | Flite | 4.882 ms | 5.008 ms | 171.3 ms | 212.5 ms |

The event guarded/direct pooled difference is about 0.13 ms for both engines.
Within-run medians vary: direct eSpeak 2.50–3.63 ms and guarded 2.85–3.26 ms;
direct Flite 4.60–5.12 ms and guarded 4.87–5.49 ms. The earlier small penalty
is not a demonstrated fixed transport cost. Do not claim equal tails or a causal
speech improvement. Four startup observations per variant are insufficient for
tail claims. Readiness still adds preparation work: roughly 41–46 ms for two lanes
relative to its direct path. Healthy preparation can precede old retirement.

Readiness capability medians are 0.975/0.998 ms direct/guarded for eSpeak and
1.044/1.056 ms for Flite. Reload medians are 6.515/6.628 ms and 3.914/3.921 ms
respectively. Total operation time is distinct from speech unavailability;
foreground requests continue during reload.

A one-second native idle observation records zero voluntary context switches for
the readiness guard versus 195 for polling. With real status output held full,
actual native tree exit after owner death takes 2.18 ms versus 6.01 ms in the
individual trials. These observations support event waiting for idle/control
behaviour, not universal recovery latency claims.

A separate interaction fixture checks sustained speech interrupted by Stop or
same-key replacement during reload, simultaneous notification speech, and 30
foreground characters per variant while the secondary worker is paused. All
four engine/owner combinations pass with both executables. Cancellation terminals
and replacement source events are retained. Some of these runs overlap build/test
activity; their timing is exploratory, not a performance acceptance result.

## Failure and client results

The original six integrated native cases pass: killed/paused owner, blocked
speech input/output, surviving helper after worker death, and guard-loss refusal.
Readiness cleanup trials take about 1.3–3.6 ms; native proof is never inferred from
these elapsed times. Additional checks establish:

- Losing a control connection preserves working speech; reconnection authenticates
  the same retained guard/member identity without creating another worker.
- An incomplete client frame expires around six seconds; speech remains usable
  and a fresh control connection succeeds.
- A full status reply path does not prevent actual owner-death observation or
  worker/helper retirement. The fixture observes `POLLOUT` interest and real tree
  exit before releasing replies, then obtains correlated tree and I/O proof.
- Partial inherited bootstrap length plus owner death retires a guard with no
  worker. A paused owner with partial bootstrap body times out around six seconds.
- A paused owner after writing only part of START retains the I/O lease. The guard
  reports tree retirement with I/O still false, then true only after actual owner
  retirement. Minimal replacement and a separate healthy Flite lane speak.

Partial writes use a test-only interposer on the actual owner; guard, worker and
helper code are unchanged. The interposer and compiler inputs are retained.
Failed fixture attempts are preserved: an early probe assumed owner exit must
precede I/O closure, another expected no legitimate receipt write, and blocked
reply draining initially deadlocked by sending its entire tail before reading.
Corrected checks require actual tree/I/O proof and deliberately denied receipt
writes; they do not weaken product cleanup.

The fresh compiled Emacs client passes paused, killed, repeated, lost-reply and
bridge-disconnect recovery with damaged configuration/history and denied session
writes. The same notification process remains usable and claims stay unchanged.
Reconnection retains uncertainty on failure and closes only the failed bridge.
No missing authority or cleanup receipt becomes proof. Client-2 timings overlap
build activity and must not be compared as a recovery speedup.

## Verification and next boundary

Pinned Rust 1.97.1 workspace tests, workspace/all-target Piper Clippy, formatting
and the three native channel regressions pass. Fresh Emacs source checks pass
29 recovery tests; the complete core suite passes 1,075 with 18 explicit skips.
Compiled-path checks confirm the changed `.elc` files are actually loaded.

Proceed with Windows job/process handles and cancellable local named pipes,
retaining native ownership before START, separate tree/I/O facts and direct speech
pipes. Native Windows execution and WSL client recovery must qualify that port;
Linux success is not a substitute. Acoustic/listening and macOS checks remain
unavailable in this slice. Keep the guard opt-in until those adoption gates are
resolved; do not revive coordinated preference transactions.
