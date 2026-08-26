# Omnivox repository workflow

- Use the exact Rust toolchain selected by `rust-toolchain.toml`; do not invoke
  an unpinned global `rustfmt`, `cargo`, or `clippy`.
- Before any mutating formatter command, run `make fmt-check`. If the committed
  baseline or untouched files fail, do not run `cargo fmt` or accept incidental
  repository-wide churn. Report the baseline problem and keep formatting work
  separate from behavioral changes.
- After the formatting baseline is clean, format only as an intentional step,
  inspect the complete diff, then rerun `make fmt-check`. Never undo formatter
  churn with a command that could discard another person's dirty changes.
- Preserve all existing tracked and untracked work. Never clean, reset, stash,
  or discard a dirty worktree to satisfy a build precondition.
- Run locked checks (`cargo test --locked --workspace` and the relevant locked
  Clippy command) so dependency resolution matches the committed lock file.
- Use `make build` or `make dev` for runnable payloads. Their build wrapper
  stages the exact generated `espeak-ng-data` and license notices beside the
  executable; a direct `cargo build` is not a complete distributable payload.
- Windows deployment is owned by the sibling Emacsvox repository. Use
  `make windows-omnivox-dev` there for a provenance-labelled build from active
  changes. Reserve `make windows-omnivox` for a clean reproducible release.
