# Lightweight preference reload: investigation result and next delivery

- Status: Investigation completed on 2026-11-10. Resident punctuation reload is implemented and passed development screening.
  Complete host preference reload has also passed development screening.
  Ordinary retained recovery and explicit minimal emergency startup are implemented in the paired client. Deployment and platform qualification
  remain separate; the staged investigation evidence is preserved unchanged.
- Scope: Independent host preference reload on ordinary owned workers, followed
  by measured recovery improvements.
- Evidence: [Measurements and verdict](../benchmarks/2026-11-10-lightweight-reload.md).
- Navigation: [Roadmap](../ROADMAP.md#host-configuration-follow-up),
  [architecture](../ARCHITECTURE.md), [decisions](../adr/README.md).

## Completed investigation

The private prototype runs on `experimental/lightweight-reload`, based on
maintenance commit `f3d6eda`. Its paired source-test client is on Emacsvox
`feat/omnivox-lightweight-reload`, based on `fb916e4f3`. Maintenance `main`, release
`v1.18.0` (`246e3b7`) and the session experiment (`7879419`) remain separate.
The original investigation plan and reassessment remain in the session worktree;
this is the maintained lightweight follow-up. Their original text is retained in
the evidence pack rather than importing session implementation or broken links.

| Checkpoint | Result |
| --- | --- |
| Smallest change and boundaries | Reuse complete startup capture and punctuation validation, existing bounded input, request state snapshots and native owners. One private management thread per enabled worker; no forwarding process or polling on speech admission. |
| One worker, then independent workers | Real punctuation preparation changes; request snapshots retain the previous policy; foreground installs and speaks while the other management connection stalls or its worker is absent. |
| Failure and responsiveness comparison | Matched release/maintenance/candidate comparisons, timestamped overlap, concurrent replacement/Stop/notifications, real Flite/Piper, retained-startup recovery, source Emacs and platform checks are recorded in the report. |
| Decision | Extend independent reload, selectively reuse lifecycle safeguards. Do not complete global session Apply as a prerequisite for preferences. |

## Private experiment contract

The retained source patches in the investigation evidence contain the experiment. An explicit random
64-hex token and a verified owned startup enable it; remote workers and ordinary
launches without the token do not expose it. A loopback endpoint is announced on
private stdout. This is a test transport, not a negotiated client interface.

The hidden candidate command captures the full selected startup configuration
without constructing engines. Each worker parses the bounded candidate away from
admission and rejects changes outside punctuation. Native startup identity stays
immutable. Live policy has a separate worker incarnation, monotonic revision and
content hash, including protection against A-to-B-to-A stale retries.

At a safe command boundary, the worker swaps a complete `Arc<PunctuationTables>`.
Previously admitted requests retain their old state; multipart reads and queued
legacy text keep their existing admission boundary. Playback need not drain.
Current voice, rate and profile selection remain intact. Removing the selected
profile rejects that worker's update. Reset uses its installed punctuation tables.
Other defaults remain as before. Recovery from retained startup can restore the
old punctuation; that is an explicit limitation of this slice.

There is one pending candidate and one reply slot per worker, bounded input of
16 MiB plus 4 KiB envelope, socket timeouts and a one-second installation wait.
A timeout means pending, not rollback or failure. Status observes actual policy;
duplicate content is unchanged, an old expected revision cannot overwrite newer
content, and a busy slot cannot accumulate more candidates. The prototype still
hashes tables on admission and needs stronger whole-frame deadline handling before
production. A legacy batch can postpone installation until dispatch or Stop;
speech remains governed by the existing protocol.

No history write, engine reconstruction, cross-worker commit or inventory scan
is part of installation. Ordinary speech, cancellation, native process ownership,
asset exclusion and audio bounds remain in their existing code paths.

## Current delivery under ADR 0017

The first delivery uses the existing local owned-worker stdio control channel,
which already crosses WSL to Windows. A resident bounded preparation task reads
the worker's selected configuration with its retained native launch context.
No new socket, token exchange, forwarding process or executable invocation is
needed per reload. Requests never supply paths, executable definitions or uploaded
configuration. Remote and unowned workers cannot acquire file-reading authority.

Each worker reports its immutable startup identity and current host preference revision
separately. Reload has bounded operation identity, observable pending/terminal
state and safe retry; stale operations cannot replay an obsolete candidate. A
successful reload swaps a precomputed immutable policy at a safe request boundary.
Status and actual installation must agree. Preparation, hashing and destruction
of large old policies stay off admission. A failed or absent notification lane
cannot roll back foreground progress. Each lane captures its own selected files;
an edit between captures may produce different lane policies, reported truthfully.

[ADR 0017](../adr/0017-independent-host-preference-reload.md) is accepted for this
independent reload contract. It extends 0008/0009 and partially supersedes
0011/0013's ordinary preference coordination; native cleanup and managed asset
protections remain applicable. No earlier accepted text was silently rewritten.

1. Make the supported local interface and live-policy status truthful across
   owners, clients, Reset and subsequent restart. Keep current startup
   acknowledgements distinct from current host preferences. Precompute policy
   hashes off admission, close framing/shutdown gaps, exercise maximum valid
   policies, and add explicit capability/version negotiation and access tests.
   Use a WSL-compatible local control channel and resident preparation: native
   Windows functionality passed, but direct loopback was unavailable and fresh
   candidate process startup took about 233 ms in the recorded setup.
2. Punctuation delivery passed; capital cues, chunking and saved Reset defaults
   now use the same bounded path with explicit wider capability negotiation.
   Native and client qualification of that extension passed; its
   [report](../benchmarks/2026-11-10-host-preferences.md) preserves the results. A mixed edit
   must be rejected or accurately classified; no silent partial installation.
   Preserve per-request coherence and temporary client choices.
3. Improve ordinary and emergency recovery separately, reusing the session
   experiment's native cleanup guards and fault tests where needed. Prove safe
   ownership after owner/guard failure before claiming dependable recovery. An
   emergency minimal engine must avoid corrupt preference/history dependencies
   while retaining proof that obsolete output cannot survive. Never erase claims
   or invent cleanup evidence.

Keep engine/model/output replacement and managed asset activation outside ordinary
preference reload. Consider a small lifecycle manager only for a demonstrated
ownership or recovery benefit. Healthy speech must not depend on its availability.
Preparing a replacement while old speech continues is a separate measured slice,
subject to native output exclusivity and memory limits.

The screening criteria remain: investigate more than 2 ms added median or 5 ms
added empirical p95 against matched release speech, more than 10 ms added p95
during reload, or any new failure/duplicate/stale output. Aim for healthy two-worker
preference completion within 100 ms empirical p95. These are proposed development
thresholds, not guarantees. Physical onset and Stop-to-silence, native macOS,
Windows client lifetime and realistic expensive-engine load remain adoption gates.

## Decisions requiring review

The accepted session decisions at `7879419` still govern adoption even though this
maintenance-based branch contains the earlier ADR index. Reconsider the global
revision/admission/completion requirements of 0011 and the mandatory coordinating
supervisor/membership assumptions of 0013. Extend 0009's frozen-preference and
Reset contract, and review the preference implications of 0008's frozen startup.
Keep 0016's managed activation separate. Retain native ownership and exclusion
protections from 0007, 0014 and 0015, along with 0001, 0003, 0004 and 0006's
isolation, cancellation and compatibility constraints. The report identifies the
scope precisely. ADR 0017 records the accepted preference-contract change and links the exact
accepted session records retained in the evidence archive.

<a id="next-recovery-experiment-and-proposed-emergency-boundary"></a>

## Emergency recovery under ADR 0018

Ordinary recovery already accepts a verified retained startup and can bypass a
broken current configuration or active pointer. It still needs readable retained
metadata and writable owner records. Explicit Emacs restart currently prepares
fresh settings. The paired client now exposes `tts-recover` and recognises
confirmed native retirement during automatic speech and character delivery.
[Native recovery evidence](../benchmarks/2026-11-10-retained-recovery.md) covers
Linux Flite/Piper and WSL-to-Windows Flite with malformed saved settings. This
ordinary route retains the native and managed-asset contracts. Lost retirement
reply testing found that the owner exits after persisting its receipt. The
[receipt verification increment](../reference/engine-startup-snapshot.md#reconcile-an-existing-retirement-receipt)
now exposes that exact proof under ADR 0007, and the paired client retains the
original host context for reconciliation. Missing or damaged proof still blocks. The
[native qualification report](../benchmarks/2026-11-10-retirement-receipts.md)
records successful Linux and WSL-to-Windows restoration after lost replies and
notification completion during foreground recovery. This does not repair lost authority
or establish the separate emergency boundary.

The minimal emergency path now addresses startup when management records are damaged
or cannot be written. It uses bundled eSpeak and a minimal in-memory startup,
bypassing preference files, optional discovery and managed-library metadata.
It has an actual native owner and bounded queues, but no managed asset load or
permission to release existing claims. Old pins,
transactions and evidence remain intact for subsequent management repair. Losing
voice selection, rate and other preferences is acceptable on this explicit route.

Do not admit emergency replacement over uncertain old output. Require actual
retirement evidence from a still-authoritative native owner, or a separately
qualified OS/reboot boundary that proves the old output process cannot survive.
Saved PIDs, absent locks and damaged journals confer no signalling authority.
Owner/worker combined failure needs its own native tests; reuse the session guard
protections where useful without importing coordinated preference commits or adding
a speech forwarding hop. No required external service manager is proposed.

The completed discriminating tests corrupt current settings with a valid
retained startup, deny management writes while a healthy worker speaks, and
exercise minimal owned startup with no management reads/writes after verified
old native retirement. Source events, command completion, a source-level recovery
gap bound and retained asset claims are recorded separately; acoustic blackout
remains unmeasured. Missing native authority still blocks replacement.
Simultaneous loss of every native authority may require an OS recovery boundary;
the minimal-startup results do not establish that boundary.

[ADR 0018](../adr/0018-minimal-emergency-speech.md) accepts the separate emergency
contract and its implementation scope. The [native/client evidence](../benchmarks/2026-11-10-emergency-speech.md)
records minimal startup and transient native proof when a durable receipt cannot
be written. The subsequent [owner-failure experiment](../benchmarks/2026-11-10-owner-authority.md)
confirms that input forwarding can block retirement and owner death can leave
either missing proof or a surviving unresponsive worker. The output-held case
can have valid durable proof despite its undelivered reply. None of these results
establishes automatic reboot reconciliation.

Preparing the actual emergency owner before retirement now removes the separate
capability process. The [matched comparison](../benchmarks/2026-11-10-deferred-start.md)
shows WSL-to-Windows median recovery falling from 579 to 393 ms and the source-gap
bound from 415 to 190 ms, with old speech preserved through preparation. Native
creation remains gated on actual old retirement, and immediate prelaunch
verification remains. Linux improves too; these are source timings, not acoustic
or owner-loss qualification.

The [independent guard extraction](../benchmarks/2026-11-10-independent-guard.md)
passes three Linux failure matrices with real Flite helpers, blocked speech and
status output, owner loss, partial START and explicit guard-loss blocking. Its
Rust guard observes tree and I/O retirement separately and adds no speech hop or
management write. This is an isolated fixture, not paired-client integration.

The [Linux guard integration](../benchmarks/2026-11-10-guard-integration.md) now
connects the actual owner, native guard and paired client. It remains opt-in.
Native faults cover owner death/pause, blocked speech input/output, a surviving
helper and explicit guard-loss blocking. The compiled client restores minimal
speech through its independent local control link with damaged management state,
preserving notifications and old claims. The first paired tests caught a lost
in-memory proof during completed-connection cleanup; a regression now preserves
that proof for explicit emergency recovery.

The [readiness and recovery follow-up](../benchmarks/2026-11-10-guard-readiness.md)
passes the integrated partial-bootstrap/START and status-backpressure matrix,
plus same-authority client reconnection. Event waiting removes idle wakeups;
320 matched character samples per engine/variant leave a small pooled guarded
source difference around 0.13 ms, within substantial run variation. Startup still
costs additional preparation, and acoustic/tail equivalence is not established.
The [Windows port investigation](../benchmarks/2026-11-10-windows-guard-qualification.md)
retains native named pipes, handle ownership and component tests, but fails the
surviving-helper completion check. Guarded Windows startup now rejects before
management/native initialization. The old child-ended plus job-accounting-zero
criterion can precede descendant process-handle completion. Transport and compiled
client successes do not override that failed proof.

The [bounded Flite handoff](../benchmarks/2026-11-10-guard-handoff-and-preparation.md)
now passes 132 native cases, including worker death during handoff and concurrent
creation. It retains actual process handles before execution and requires their
completion. The result supports a Windows creation service but does not qualify
the current product guard. Retained ADR 0015's completion criterion and the
compatibility of child-creating helpers need explicit review before adopting that
boundary. No accepted decision has been rewritten.

Concurrent owner/control-bridge preparation is implemented independently in the
client. Matched Windows emergency-call medians fall from 876 to 696 ms. A focused
20-per-variant healthy-restart follow-up shows source-gap bounds around 181/184 ms;
it does not establish acoustic or tail non-regression. Both
old lanes continue source speech during repeated preparation and preference
reload. Preserve the existing steady-speech baseline evidence.

Next, decide the bounded Rust creation interface and compatibility scope: one
actual Flite helper first, direct PCM pipes, verified launch inputs, actual process
handles and cancellation through failed transfers. Resolve child-creating helpers
before broader enablement; do not silently exclude them. Follow the ADR approval
workflow for a durable ownership change. Keep Windows guarded startup unavailable
until native proof is integrated and qualified. Preserve ordinary preference
reload independently. Native macOS and acoustic/listening checks remain separate
gates; the guard is not enabled by default.
