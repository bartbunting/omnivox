# Extensible engine configuration: implementation checks and deadline gap

Functional development evidence, 2026-09-27. This is not a performance baseline,
release qualification or acoustic measurement. Configuration v1 implementation
is incomplete while the startup ownership gap below remains unresolved.

## Provenance and coverage

[Provenance](data/2026-09-27-engine-framework/provenance.json) records Omnivox
`48921d0d5edc33e60815e935acd2beaace809257`, paired Emacsvox
`1ff3f466b87ebe32c8022d3f98d37dc756e9d5a3`, the pinned Rust/Emacs versions,
Linux/WSL host and staged development binary digest. The deadline reproduction
adds the two explicitly ignored tests in
[`initialization_tests.rs`](../../omnivox-tts/src/helper_engine/initialization_tests.rs).
The fixtures execute no user runtime and release their blocked operations after
500 ms, including on a failed assertion. Raw artifacts have
[checksums](data/2026-09-27-engine-framework/SHA256SUMS).

The implementation includes strict bounded configuration/manifest readers,
common shipped/external launch resolution, frozen native environments and
managed inputs, local selection permissions, complete owner-to-worker snapshots,
independent worker acknowledgements, paired Emacsvox startup/Apply/recovery, and
workstation-local remote session snapshots. No configuration editor, engine
installation service, richer language routing or release publication is added.

| Check | Observed result | Retained output |
| --- | --- | --- |
| Locked Rust workspace tests before the gap fixtures | Passed. | [Compressed log](data/2026-09-27-engine-framework/workspace-tests.log.gz) |
| Locked workspace tests and all-target Clippy with the gap fixtures | Passed; the two known failing deadline regressions remain explicitly ignored. | [Tests](data/2026-09-27-engine-framework/workspace-with-gap-fixtures.log.gz), [Clippy](data/2026-09-27-engine-framework/workspace-with-gap-fixtures-clippy.log) |
| Workspace all-target Clippy | Passed with warnings denied. | [Log](data/2026-09-27-engine-framework/workspace-clippy.log) |
| CLI Piper feature and Windows GNU cross-target Clippy | Passed with warnings denied; cross-compilation is not native qualification. | [Piper](data/2026-09-27-engine-framework/piper-clippy.log), [Windows](data/2026-09-27-engine-framework/windows-cross-clippy.log) |
| Fresh compiled Emacs, local real workers, null output | 9 tests passed. Both workers acknowledged one snapshot after files changed; recovery retained it and the other lane; deliberate fresh startup rejected invalid input. | [Log](data/2026-09-27-engine-framework/emacs-local-compiled.log.gz) |
| Focused Emacs runtime/local/remote tests | 195 passed; the separately exercised opt-in local native test skipped in this run. | [Log](data/2026-09-27-engine-framework/emacs-remote-focused.log.gz) |
| Broker process suite, including fresh compiled Emacs | 6 passed; optional selected vendor engine and 20-second lease tests skipped. Covers shared UUID, changed files, total disconnect, one-lane recovery, and new-session restart. | [Log](data/2026-09-27-engine-framework/remote-compiled-process.log) |
| Emacs documentation and attribution gates | Passed after byte-code rebuild/update. | [Documentation](data/2026-09-27-engine-framework/emacs-documentation.log), [headers](data/2026-09-27-engine-framework/emacs-headers.log) |
| Full Emacs suite at the local integration slice | 3836 expected results, 26 skips, one unrelated startup-inventory failure. | [Compressed log](data/2026-09-27-engine-framework/emacs-full-tests.log.gz) |

The full-suite failure is
`emacsvox-setup-tracks-maintained-runtime-inventory`: the existing Lisp build
includes `emacsvox-aural-voice-bulk.el`, but the startup source guard omits it.
Neither its test, the Lisp build inventory nor the guard was changed by these
implementation slices. It remains unfixed and is not counted as a passing gate.
The full suite predates the final remote client slice; the focused and process
checks above exercise that later slice.

## Reproduced startup deadline gap

The external initializer uses four scoped threads and joins them before returning
the inventory. `initialize_before` invokes the connector synchronously. Its
protocol watchdog can terminate a child to unblock a write, but if termination
fails, the initialization call still waits for that write. Neither case lets the
batch return eligible ordinary engines at its configured deadline.

One controlled observation per case used Rust `Instant` around
`initialize_before`, a 100 ms admission budget and a separate 500 ms fixture
release. [Raw reproduction output](data/2026-09-27-engine-framework/startup-deadline-gap.log)
records:

| Controlled fault | Return time | Expected property |
| --- | --- | --- |
| Connector does not return until released | 504.066516 ms | Admission should stop waiting while the unfinished attempt remains owned. |
| Hello write remains blocked after cleanup initially fails | 500.217589 ms | Unconfirmed cleanup should retain ownership and forbid replacement without blocking ordinary startup. |

Both regression assertions fail when explicitly enabled. They are ignored in
the ordinary suite with a specific known-gap reason, not counted as successful
deadline coverage. Reproduce from the repository with the pinned toolchain:

```sh
cargo +1.97.1 test --locked -p omnivox-tts startup_deadline_does_not_wait -- --ignored --nocapture --test-threads=1
```

The observed return times follow the artificial release; they are not measured
OS spawn delays. No actual kernel stall or failed OS process termination was
induced. Existing successful-cleanup tests still cover bounded Hello/Describe
reads and writes and retirement of a real stalled Unix helper.

## Remaining acceptance

The fix must preserve the four-slot bound across unfinished attempts, retain
cleanup ownership, refuse replacement until retirement is confirmed, prevent
late descriptors from becoming available, and handle owner drop without a
self-join or losing a child. Simply dropping the scoped thread handles does not
establish those properties. The accepted configuration contract is unchanged.

Native Windows full development staging and qualification, completion of the
framework acceptance matrix, and final roadmap/changelog reconciliation remain
outstanding. No Windows/macOS native or audible acceptance is claimed here.
