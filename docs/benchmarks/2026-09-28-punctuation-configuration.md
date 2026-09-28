# Punctuation configuration acceptance

Stage one of the [punctuation plan](../plans/punctuation-configuration.md) adds
per-level names and preservation to local configuration. The
[reference](../reference/engine-configuration.md#punctuation-tables) specifies
schema 3 and the complete default tables. This work is unreleased.

## Observed results

The following checks passed on Linux with pinned Rust 1.97.1:

- Locked workspace tests, including exhaustive ASCII behavior, contractions
  with straight/curly/modifier apostrophes, common Unicode defaults, custom
  names, preservation and nonrecursive expansion. Structured tests retain every
  requested UTF-8 boundary, unsorted/repeated offsets and the `[*]` separator.
- Strict parser checks reject invalid scalars, unknown levels, duplicate decoded
  keys, invalid names, excessive resolved entries and null outside character
  values. Historical startup schemas retain their original shape and ASCII
  behavior; schema 6 requires resolved tables.
- Locked workspace Clippy with all targets, both with and without the CLI Piper
  feature; `make dev` stages the runnable executable and its eSpeak data.
- The process harness records actual helper requests for all three levels,
  custom names and preservation, plus previews. Two owned workers share one
  captured snapshot. File/environment mutation, client reset and retained-owner
  recovery leave pronunciations unchanged. Fresh activation rejects the changed
  invalid file. Independent retirement and blocked-write cleanup pass.

The two ignored workspace tests require explicitly qualified native helper paths
or a long synthesis stress run; they are not recorded as passes. The process harness uses null output and a
Python helper generating fixture PCM. It proves text and lifecycle behavior,
not natural voice pronunciation, physical sound or latency. No live speech
profile was restarted. Windows/macOS and live Emacs listening were not exercised
for this feature. The original reporter's exact session was not reproduced.

## Reproduction and provenance

The development worktree is based on `4aabd38048613ad5408d8b31eca76e25aa306637`.
[Retained records](data/2026-09-28-punctuation-configuration/) identify the source
diff, executable, toolchain, platform and command results. The source diff hash
covers maintained Rust changes and the process harness, including new files;
documentation is excluded. The process harness removes its temporary request
traces after success. Its fixture and assertions remain in
[`verify_engine_configuration.py`](../../tools/verify_engine_configuration.py).

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --features omnivox-cli/piper -- -D warnings
make dev
python3 tools/verify_engine_configuration.py target/debug/omnivox
make fmt-check
make docs-check
make docs-check-paired
```

The paired Emacsvox change records the delivery boundary in its developer
backlog; its `make docs-release-check` passed. No Emacsvox Lisp change is needed
for the existing three levels. Named-profile negotiation remains deferred.
