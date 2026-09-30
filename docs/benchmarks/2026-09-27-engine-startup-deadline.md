# Engine startup deadline: retained attempts and caller publication

Functional development evidence, 2026-09-27, Linux/WSL x86-64. This report
supersedes the outstanding deadline finding in the
[earlier framework report](2026-09-27-engine-framework.md#reproduced-startup-deadline-gap).
Its original observations and artifacts remain unchanged. This is not native
Windows qualification, a performance baseline or an acoustic measurement.

## Change and coverage

External initialization now waits on an owned task. The task retains the engine,
its lifecycle lock and one of four process-wide admission slots until its work
finishes. A caller can return at the batch deadline while blocked launch or
protocol work remains owned. A retry joins a finished attempt before reusing the
engine; a completed task with unconfirmed connection cleanup leaves that
connection on the retained engine and still forbids replacement.

The task offers a negotiated connection to its caller and waits for acceptance.
Only the caller publishes, after acquiring the publication locks within its
deadline. An expired caller or dropped result declines the candidate and causes
retirement. Dropping the host's last reference does not drop a blocked attempt:
its worker retains the engine through completion and final cleanup without
joining itself. Native runtimes remain in their existing helper processes.

The two previously ignored regressions now run in the ordinary test suite.
One recorded observation per case uses Rust `Instant` around initialization,
a 100 ms admission budget and a separate 500 ms fixture release. The
[helper test log](data/2026-09-27-engine-startup-deadline/helper-tests.log.gz)
records both return times as 100 ms, truncated to whole milliseconds. The old
observations returned at approximately 500 ms. These controlled checks establish
that the caller no longer waits for the artificial release; they do not measure
kernel spawn latency or establish a timing distribution.

The same run covers:

- Four blocked attempts retain all slots after the batch returns. Neither a
  retry on the same owner nor a fifth helper starts until they finish.
- A fully negotiated candidate blocked at descriptor publication times out,
  retires and never becomes available after the publication lock is released.
- A host can drop its last reference while a connector is blocked; the worker
  cleans up the late child and finishes without retaining or joining itself.
- Failed Hello-write cleanup blocks replacement while the old writer is live.
- Worker panics return failures and release their slots; a later healthy
  initialization succeeds.
- Existing Hello/Describe read/write watchdog cases, retained failed cleanup,
  explicit recovery, cancellation and a real stalled Unix child remain covered.

## Verification and provenance

[Provenance](data/2026-09-27-engine-startup-deadline/provenance.json) records
Omnivox `7cdf056164e8e18b1a62b991cb287425ff01afc5`, its base commit, tested-source
digests, staged binary digest, pinned toolchain, host and commands.
[Checksums](data/2026-09-27-engine-startup-deadline/SHA256SUMS) cover the retained
raw logs. The tests use mock connectors/connections except the explicitly named
real Unix process cases. Fixtures release their blocked operations and verify
retirement; no actual failed OS termination or stalled kernel spawn was induced.

| Check | Observed result | Retained output |
| --- | --- | --- |
| Helper-engine tests | 62 passed, none ignored; includes both original deadline regressions. | [Log](data/2026-09-27-engine-startup-deadline/helper-tests.log.gz) |
| Locked workspace tests | Passed. | [Log](data/2026-09-27-engine-startup-deadline/workspace-tests.log.gz) |
| Locked workspace all-target Clippy | Passed with warnings denied. | [Log](data/2026-09-27-engine-startup-deadline/workspace-clippy.log) |
| CLI Piper and Windows GNU cross-target Clippy | Passed with warnings denied. | [Piper](data/2026-09-27-engine-startup-deadline/piper-clippy.log), [Windows](data/2026-09-27-engine-startup-deadline/windows-cross-clippy.log) |
| Staged development payload | `make dev` passed, including eSpeak data and notices. | [Build log](data/2026-09-27-engine-startup-deadline/development-build.log.gz) |
| Real local owners and fake helpers, null output | Shared preparation, independent acknowledgements/audio targets, frozen inputs, fresh activation, blocked startup transmission and retirement passed. | [Log](data/2026-09-27-engine-startup-deadline/local-owner-process.log) |

Full Windows development staging and native qualification, the complete
framework acceptance matrix and final roadmap/changelog reconciliation remain
outstanding. Paired Emacsvox code is unchanged by this fix; the earlier report
retains its compiled-client coverage and unrelated full-suite inventory failure.
