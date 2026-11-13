# Lightweight reload source integration

This report records local integration of the paired preference reload,
guarded recovery and early-speech branches on 2026-11-13. It is functional
merge evidence, not a release, desktop deployment or acoustic measurement.
The [delivery plan](../plans/lightweight-preference-reload.md) retains the
outstanding platform and listening work. Guarded ownership and early optional
discovery remain opt-in.

## Source and resolution

Omnivox combines `main` at `f025849` with `experimental/lightweight-reload`
at `7ac4f5d`. Its executable sources are identical to the feature branch.
The removed pause/resume backlog item stays removed. Current status wording
now identifies development main while preserving historical evidence.

Emacsvox combines `master` at `89acd0601` with
`feat/omnivox-lightweight-reload` at `0acf2deae`. The Lisp merge preserves
master's Apply failure handling and the feature's late-registration behavior:
lane roles survive process exit, observer errors cannot suppress completion,
and unacknowledged newer registry writes prevent reuse of an old generation.
Existing regression tests cover these paths.

The UI plan conflict keeps master's concise active plan, adding only remaining
reload/recovery acceptance work and a link to the paired delivery plan. Info
conflicts were resolved by generating the manuals from the combined source.

Exact parents, the Emacsvox candidate tree, toolchains, runtime identity and
input locations are in [provenance](data/2026-11-13-lightweight-merge/provenance.json).
The [artifact checksums](data/2026-11-13-lightweight-merge/SHA256SUMS)
cover retained raw logs, native results, source hashes and reproduction scripts.
Compressed artifacts preserve their original bytes. Session snapshots containing
environment values and personal speech logs are not included.

## Verification

- Rust 1.97.1: `make fmt-check`, `cargo test --locked --workspace`
  (1,218 passed, two ignored), and workspace/all-target locked Clippy with
  warnings denied passed. Windows helper and remote SSH harness Python checks
  passed 16 tests; the latter uses its local harness, not a production host.
- Linux Emacs 31.0.90: all 377 maintained modules compiled; bytecode preflight,
  compiled aural parity and compiled Notmuch checks passed. Notmuch ran 142
  passing tests and skipped one graphical test. The pinned graphical menu
  suite passed all 287 tests in a private X display.
- The full source suite ran 4,734 tests: 4,482 passed, four failed and 248
  were skipped. Manual navigation passed when rerun after regeneration.
  Three remaining failures also occur on pre-merge master: the aural reference
  is stale, the Windows guide test expects obsolete exact ARM64 wording,
  and the stderr-rotation test misses its 1.5-second live-output deadline in
  this environment. The reference preview identifies existing tree and wrapped
  line metadata absent from the generated reference. These unrelated files
  were left unchanged; this report does not claim an entirely green source suite.
- `make docs-generate`, `make docs-release-check` and `make headers-check`
  passed for Emacsvox. Omnivox documentation, paired documentation and whitespace
  checks passed before the merge commits.

## Native Windows checks

An isolated candidate used native Emacs 31.1. Its 1,044 copied tracked source
files matched the merged checkout; native compilation and preflight passed
for 377 modules. The borrowed installation metadata selects runtime locations
only: its old source commit is not the merged client's provenance.

The existing qualified runtime, build `4e1c1561f51908a6`, was reused because the
merged Omnivox executable source is unchanged from the feature branch. Its
SHA-256 is recorded in provenance. The desktop shortcuts and their installed
clients were not replaced.

Five null-output native runs passed all eight steps: ordinary Eloquence,
early-discovery Eloquence, DECtalk, WinRT and Flite. Each checked two guarded
lanes, speech during delayed registration acknowledgement, in-place preference
installation, malformed settings and retained recovery, control reconnection,
owner loss and emergency recovery, return to saved startup, and foreground
reload without notifications. Separate graphical personal-profile checks
passed with Eloquence and DECtalk on both lanes, current registry acknowledgements,
unchanged preference reload and retirement of the test-owned processes.
These are automated execution checks, not new listening acceptance.

## Prerequisite failures and limits

Initial Rust attempts lacked staged helper inputs, then used the wrong Piper
directory. With verified inputs, a long build path exceeded the bundled eSpeak
phoneme compiler's path limit. A fresh shorter target directory resolved that
environment problem without source or dependency changes. Initial native client
compilation lacked the root `VERSION` file; adding the tracked root files
resolved it. Failed logs remain alongside the successful runs.

No new native macOS qualification, acoustic timing, package-loading optimization
or release claim follows from this merge. The installed Windows and WSL
development launchers remain separate from the merged source branches.
