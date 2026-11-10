# Replacement and handover evidence

The [report](../../2026-11-10-replacement-and-handover.md) separates implementation,
source timing, native failure checks and outstanding listening/platform work.
Nothing here is a release payload. Runtime binaries and user-supplied engines
are excluded from these archives; their hashes and external paths are retained.

## Contents

- `observations.tar.gz`: raw Windows phase, reload/character, restart, delayed
  notification, selected Eloquence/DECtalk restart, native isolation and fresh
  profile observations, including failed attempts; Linux Flite phase observations.
- `checks.tar.gz`: full staging, rejected reuse, builds, formatting, tests,
  Clippy, byte-code and documentation logs. Failed attempts are retained.
- `archive-inventory.json`: individual archived hashes and excluded binary hashes.
  The assembly script rereads and verifies every member before reporting success.
- `phase-summary.json`, `engine-summary.json`, `restart-summary.json` and
  `restart-engines-summary.json`: derived source observations, with raw samples
  retained in the archive. Medians pool the stated cells; p95 uses nearest rank.
- `runtime-path.json`, `provenance/`, source patches and companion comparisons:
  exact private payload and source identity. The staged patch SHA-256 matches
  Omnivox's recorded dirty-worktree hash. The committed patch adds formatting
  and tests to that same executable behaviour.
- `fixtures/`: reproduction inputs, including the initial failed selected-engine
  fixture. Imports reuse the retained resident/emergency probes in this repository.
- `listening/`: isolated private-profile launcher, pinned runtime metadata and
  startup fixture. Interactive launch uses real output; `--batch-check` selects
  null output and asserts reload, restart, retained recovery, emergency and return.

## Reproduction

These fixtures require the recorded native Windows x64 runtime and external
licensed runtimes on this workstation. Update explicit executable/data paths to
another verified payload; never direct the probes at installed settings. Output
roots must be new. Run from the Omnivox repository. The private Emacs source path
and exact Emacs 31.0.90 executable are explicit in the client fixtures.

`flite-phase.py EXECUTABLE NEW_ROOT` reads `OMNIVOX_TEST_WINDOWS_DATA`,
`OMNIVOX_TEST_RHVOICE_LIBRARY` and `OMNIVOX_TEST_RHVOICE_DATA`. It defaults to two
alternating blocks across minimal/full profiles. `PROFILES=minimal REPEATS=1`
also runs Linux against the complete `make build` payload; the data variable
then names that payload's Linux eSpeak data directory.

`REPEATS=4 SAMPLES=40 SAMPLES_WINRT=160 python3 fixtures/engines.py
runtime-path.json NEW_ROOT` reproduces the eight-engine character/reload matrix.
All engines are checked in inventory and actual speech. No unavailable preferred
engine or fallback counts as successful selected-engine measurement.

`restart-availability.py EXECUTABLE NEW_ROOT` uses fresh compiled Emacs and
continuous tracked speech on both lanes. Set `OMNIVOX_TEST_BYTECODE=1` and the
three native-path variables above. For the matched baseline, set
`OMNIVOX_TEST_STARTUP_BASELINE` to the separately compiled previous `tts-speak.el`;
its source, commit, compiler and byte-code hash are retained under `provenance/`.
The comparison order was baseline, candidate, candidate, baseline, four restarts
per fresh profile. `OMNIVOX_TEST_NOTIFICATION_DELAY=2` adds the deliberate delay.
`restart-engines.py ... --engine eloquence` or `--engine dectalk` additionally
asserts every observed source engine. Keep the initial failed fixture separate.

The successful Linux workspace checks used:

```sh
env CARGO_TARGET_DIR=/home/bart/tmp/ovlw-check \
  OMNIVOX_PIPER_INPUTS_DIR=/home/bart/src/omnivox/target/piper-inputs/1.7.0/x86_64-unknown-linux-gnu \
  cargo test --locked --workspace
env CARGO_TARGET_DIR=/home/bart/tmp/ovlw-check \
  OMNIVOX_PIPER_INPUTS_DIR=/home/bart/src/omnivox/target/piper-inputs/1.7.0/x86_64-unknown-linux-gnu \
  cargo clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
```

Run these from the repository so `rust-toolchain.toml` selects Rust 1.97.1.
`make build` stages Linux runtime data and helpers. Paired `windows-omnivox-dev`
staged the Windows payload through the recorded private destination; the reused
companions are byte-identical to the previous build. Windows CLI Clippy and test
compilation used the pinned image and GNU Windows target; eighteen isolation
tests then ran on native Windows. Native guard lifetime tests from the preceding
unchanged implementation remain linked evidence, not newly claimed reruns.

Emacs validation used the named Emacs 31 executable, focused ERT, incremental
byte-code/preflight, the full suite and `docs-release-check`. The retained
comparison shows the same seventeen baseline failures, rather than claiming a
clean full suite. See the report for native, acoustic and listening limits.
