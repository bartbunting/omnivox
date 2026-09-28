# Local punctuation editor acceptance

The unreleased editor extends the first part of the
[punctuation plan](../plans/punctuation-configuration.md). Omnivox owns defaults,
validation and local file writes; Emacsvox provides `M-x omnivox-punctuation`
and the Aural Home entry. Named profiles remain deferred. The
[editing contract](../reference/engine-configuration.md#local-punctuation-editor)
defines the private service and save boundary.

## Environment and checks

Linux x86-64 on WSL2, pinned Rust 1.97.1, Emacs 31.0.90 and an isolated
1280-by-800 Xvfb display were used. Each final command ran once after the
relevant fix. These are functional checks, not timing benchmarks; ERT reports
wall-clock durations but no physical audio timing was measured.

- Locked workspace tests and workspace Clippy with all targets and the CLI
  Piper feature passed. Two workspace tests remain explicitly ignored for
  native-helper/stress prerequisites; they are not passes.
- `make dev` staged the executable with its eSpeak data. The process harness
  passed review, Unicode save, stale-write rejection and preservation of unrelated
  settings, alongside the existing two-worker snapshot and recovery checks.
- Rust tests cover read-only review, strict draft validation, independent target
  identities, locking, permission preservation, missing default roots and refusal
  to replace a linked configuration file.
- A fresh Emacs loaded the compiled editor and passed six tests including an
  actual private-service round trip. Its graphical test was skipped in batch
  and passed in the separate graphical suite: all 38 tests passed.
- The graphical runner initially retained its previous expected count of 37;
  its first run passed all 38 assertions but returned failure. The count was
  corrected and the gate rerun. The core suite also identified a missing startup
  inventory entry for the new module; that entry was added before the final run.
- The final core run passed 923 tests with 12 prerequisite skips and no
  unexpected results. Aural UI/Home checks passed 129 tests after loading their
  voice-editor dependency; the initial standalone invocation omitted it and
  reported two unbound-keymap errors. No unrelated source was changed.

The new graphical test checks visible rows, selected-column retention, spoken
row content, spoken-help dispatch, help's file path, exact return position and
draft retention. Speech calls are captured, not played. The native-service test
uses isolated temporary configuration and voice roots. The process harness uses
null output and a Python helper generating fixture PCM. No live profile was
changed, no acoustic listening was performed, and this feature has not received
native Windows or macOS acceptance. WSL-to-Windows uses the existing launcher
route but requires an updated native host. An older host fails with an update
message. Remote and direct native-Windows Emacs providers are outside this UI.

## Reproduction and provenance

[Retained records](data/2026-09-28-punctuation-editor/) identify base commits,
source-diff hashes, executable hash and check results. Diff hashes cover source
and test changes, including added files; documentation is excluded. This is
uncommitted development evidence, not release qualification.

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
make dev
python3 tools/verify_engine_configuration.py target/debug/omnivox
make fmt-check
make docs-check
make docs-check-paired
```

In Emacsvox, run `make bytecode-rebuild`, then set
`OMNIVOX_PUNCTUATION_TEST_PROGRAM` to the staged Linux executable for
`make core-test`. Run `make graphical-voice-test` separately. The maintained
manual and generated references were updated and checked with
`make docs-release-check`.
