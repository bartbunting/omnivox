# Omnivox repository workflow

- Before changing architecture, engine process boundaries, helper protocols,
  release contents, or packaging policy, read every architecture decision
  record under `docs/adr/` and follow all accepted decisions. Do not rely on a
  single record in isolation; later records may refine earlier decisions.
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
- Treat every `CHANGELOG.md` section named by an existing release tag as frozen
  release history. Before editing the changelog, inspect the tags and compare
  the relevant tag's commit ancestry with `HEAD`; work committed after that tag
  belongs under `Unreleased`. A workspace version or prepared version heading
  is not evidence that later work shipped. Promote `Unreleased` entries into a
  dated version section only during explicit release preparation for the commit
  that will be tagged, and then recreate an empty `Unreleased` section. Never
  add post-tag work to an older release section or rewrite a tag to make the
  changelog fit; factual corrections to published history must remain clearly
  corrections rather than claims that later code was released.
- Run locked checks (`cargo test --locked --workspace` and the relevant locked
  Clippy command) so dependency resolution matches the committed lock file.
- Use `make build` or `make dev` for runnable payloads. Their build wrapper
  stages the exact generated `espeak-ng-data` and license notices beside the
  executable; a direct `cargo build` is not a complete distributable payload.
- Windows helper source and build targets are owned under `windows-helpers`;
  preserve their GPL-2.0-or-later notices and separate executable boundary.
  Final Windows deployment is owned by the sibling Emacsvox repository. Use
  `make windows-omnivox-main-dev` there when its guard accepts a main-server or
  main-only audio-output change and a verified development runtime is already
  staged. Use `make windows-omnivox-dev` when that guard rejects helper,
  protocol, dependency, toolchain, companion, or shared-code changes. Reserve
  `make windows-omnivox` for a clean reproducible release that rebuilds every
  payload.
