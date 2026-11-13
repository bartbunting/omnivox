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

The initial private prototype ran on `experimental/lightweight-reload`, based on
maintenance commit `f3d6eda`. Its paired source-test client used Emacsvox
`feat/omnivox-lightweight-reload`, based on `fb916e4f3`. Those names identify the
investigation baselines alongside release `v1.18.0` (`246e3b7`) and the separate
session experiment (`7879419`). The subsequent implementation is integrated
into Omnivox `main` and Emacsvox `master`; guarded ownership and early optional
discovery remain opt-in, with the qualification limits below unchanged.
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

The accepted session decisions at `7879419` still govern adoption even though
development main contains the earlier ADR index. Reconsider the global
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
surviving-helper completion check. That implementation was gated before
management/native initialization. The old child-ended plus job-accounting-zero
criterion can precede descendant process-handle completion. Transport and compiled
client successes do not override that failed proof.

The [bounded Flite handoff](../benchmarks/2026-11-10-guard-handoff-and-preparation.md)
now passes 132 native cases, including worker death during handoff and concurrent
creation. It retains actual process handles before execution and requires their
completion. The result supports a Windows creation service but does not qualify
the current product guard. [ADR 0019](../adr/0019-windows-native-creation-ownership.md)
now accepts the narrower creation boundary and refines the retained completion
criterion. Child-creating helpers remain outside its initial qualification scope.

Concurrent owner/control-bridge preparation is implemented independently in the
client. Matched Windows emergency-call medians fall from 876 to 696 ms. A focused
20-per-variant healthy-restart follow-up shows source-gap bounds around 181/184 ms;
it does not establish acoustic or tail non-regression. Both
old lanes continue source speech during repeated preparation and preference
reload. Preserve the existing steady-speech baseline evidence.

The [Rust creation components](../benchmarks/2026-11-10-windows-owned-creation.md)
and [integrated Windows qualification](../benchmarks/2026-11-10-windows-creation-integration.md)
now support an opt-in eSpeak/Flite prototype. Native owner/worker/helper failures,
blocked forwarding, lost replies, malformed management state, compiled client
recovery and direct speech during reload have retained evidence. Confirmed helper
launch rejection preserves the worker; uncertain transfer still retires it.
The integrated report separates ordinary response, source timing and startup cost,
including the smaller helper-pipe capacity found and corrected during comparison.

The [Windows multi-engine slice](../benchmarks/2026-11-10-windows-multi-engine.md)
extends qualification to WinRT, RuTTS, TGSpeechBox, Eloquence, DECtalk and RHVoice.
Each of six concurrently initialized helpers can fail and recover independently.
The guard uses at most eight retained helper slots and one creation/handoff at a
time. Console-free .NET helper builds and compatible executable-path handling
preserve child restrictions without changing helper protocols. The client also
overlaps bridge startup with ordinary owner preparation. The report separates
startup spans, reload responsiveness and source availability from acoustic onset.
The [replacement and handover follow-up](../benchmarks/2026-11-10-replacement-and-handover.md)
resolves the measured Flite and foreground blockers with two small changes.
Flite asks for cancellation promptly while retaining its native slot until actual
completion; 480 Windows replacements during reload complete in approximately
14–18 ms median across phases, with cooperative helper reuse. Other engines retain
their existing cancellation policy. The expanded character comparison passes the
proposed +5 ms guarded p95 criterion for all eight engines, including 640 WinRT
samples per path; this is a workload result, not an acoustic or universal tail bound.

The compiled client prepares foreground voices before retiring old foreground,
then configures notification independently. Matched median main source gap falls
from 1,473 to 79 ms; a two-second notification delay leaves a 58 ms main gap.
Full restart still takes about 3–4 seconds with eight engines. Preparation failure
preserves old speech. Managed activation and native cleanup boundaries are unchanged.
The new private profile passes reload, restart, retained recovery, minimal emergency
speech and return to saved settings with all eight engines.

Interactive listening exposed two follow-up startup failures. The
[client acknowledgement correction](../benchmarks/2026-11-10-foreground-registration.md)
repairs a late registry reply after a settings timeout. The
[Windows queue correction](../benchmarks/2026-11-10-helper-queue.md) prevents
healthy concurrent helpers from exhausting a single creation-phase deadline
while waiting for their turn. The current private profile also translates WSL
sound paths for the Windows worker. These fixes passed isolated source and
recovery checks. The user subsequently confirmed that Eloquence works in the
corrected prototype; broader interaction and recovery listening checks remain
separate.

The [native Windows client follow-up](../benchmarks/2026-11-13-native-windows-client-reload.md)
separates local ownership negotiation from the POSIX engine-file provider. Native
Emacs now exercises independent preference reload and guarded recovery using the
same qualified Windows runtime. Compiled WinRT/Flite/Eloquence/DECtalk lifecycle checks and the
full graphical personal profile pass. An additional early-discovery run exposed
a registry-generation admission race. The
[early-start follow-up](../benchmarks/2026-11-13-native-early-start.md) corrects
unnecessary generation changes while preserving newer unconfirmed writes.
Final native WinRT/Flite/Eloquence/DECtalk checks, the full Windows profile and
WSL Eloquence/DECtalk checks pass with early startup. The native launcher now
defaults to Eloquence, matching Development; shared startup files were already
identical. Acoustic timing and new early-start listening acceptance remain separate.

Next slices:

1. The Development shortcut now uses the
   [normal personal profile](../benchmarks/2026-11-11-personal-development-launch.md)
   with the qualified runtime. Bart's
   [2026-11-13 daily-use confirmation](../benchmarks/2026-11-11-personal-development-launch.md#daily-use-confirmation-2026-11-13)
   closes general interactive acceptance for that setup: he reports continued use
   with no issues. Per-scenario coverage was not itemised. Use the
   [isolated listening profile](../benchmarks/data/2026-11-10-early-speech-refresh/listening/start.py)
   for targeted real-output checks of reload, restart and recovery, including
   concurrent navigation, replacement and notifications. Physical timing and
   device recovery remain separate qualification gaps.
2. Qualify remaining engines and platforms: Windows Piper needs its model and
   companion; MBROLA and external wrappers need supported child creation and
   native qualification. Preserve their ordinary launch paths. Qualify native
   macOS separately; no automatic installed deployment.
3. The [startup investigation](../benchmarks/2026-11-10-startup-investigation.md)
   found selected Flite ready 4.75 seconds before command admission in a slow
   launch; removing TGSpeechBox alone gave no consistent warm improvement.
   The opt-in [early-speech prototype](../benchmarks/2026-11-10-early-speech.md)
   now passes delayed discovery, fallback, active cancellation, retirement,
   independent notification and compiled client late-selection checks. Warm
   first-source median improves from 1,137 to 864 ms in six matched runs per path.
   The [paired client follow-up](../benchmarks/2026-11-10-early-speech-refresh.md)
   qualifies saved preferences targeting a pending engine, automatic per-lane
   observation, newer choices, optional failure and prepared restart. The private
   launcher reuses Emacs native code caches while preserving isolated settings.
   Before public adoption, review the readiness and pending-discovery contract. Existing complete-inventory managed startup remains unchanged.
   Foreground availability now survives the longer notification startup;
   reducing total operation time is a separate objective. Any further engine's
   cancellation exception needs its own replacement, native retention and failure
   evidence. Keep preference reload independent and avoid a new coordination layer.

Preserve ordinary preference reload independently. Loss of all cleanup authority,
reboot reconciliation and broader asset-management recovery remain distinct work.
The guard remains disabled by default; this milestone does not close every
platform, performance or reliability acceptance criterion.
