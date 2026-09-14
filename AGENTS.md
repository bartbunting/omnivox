# Omnivox repository workflow

## Working agreement

- Complete authorized implementation and verification. Resolve routine choices
  from repository evidence and reasonable assumptions; do not stop at a plan
  or ask repeatedly for permission to finish the requested work.
- The global approval boundaries still apply. Ask before crossing a boundary
  unless the conversation already authorizes that action; an implementation
  request does not authorize unrelated changes.
- When clarification is necessary, ask a focused question and continue work
  that does not depend on the answer. If an instruction blocks progress,
  identify its file, quote the exact rule, and explain the concrete conflict.
  Distinguish an explicit requirement from your interpretation.
- Keep updates and final reports concise. Distinguish verified results from
  assumptions, and identify checks that could not run and why.
- Delegate only when the active session permits it and a bounded independent
  task benefits from it. Delegation is optional, not a completion requirement.

## Reasoning effort

- Assess the reasoning level before substantial work and when complexity
  changes. Recommend a switch when the expected quality or time benefit
  justifies interrupting the task, rather than for every small step.
- Use these repository heuristics with levels available in the current session:
  Low for straightforward documentation or mechanical edits; Medium for bounded
  implementation with clear requirements; High for difficult debugging or
  changes spanning crates; Extra high (`xhigh`) for subtle concurrency,
  streaming, cancellation, protocol compatibility, or architecture decisions;
  Max for exceptionally difficult analysis that still has unresolved competing
  explanations or interacting constraints after focused investigation.
- Consider a lower level when the difficult analysis is complete and substantial
  routine implementation remains. Respect the user's chosen level; revisit a
  declined recommendation only when new evidence materially changes the task.
- State the current level only if the session exposes it; otherwise say it is
  unknown. Do not infer it from apparent task difficulty or configuration defaults.
- When recommending a change, name the proposed level, explain the concrete
  reason, and briefly record progress and the next step. Then stop and ask the
  user to change the reasoning control in Codex. Resume only after the user
  confirms the change or explicitly asks to continue at the current level.
  Do not change settings yourself or claim that a request changed the level.
  This pause is an explicit exception to autonomous follow-through above.

## Architecture and worktree

- Before changing architecture, engine process boundaries, helper protocols,
  release contents, or packaging policy, read every architecture decision
  record under `docs/adr/` and follow all accepted decisions. Do not rely on a
  single record in isolation; later records may refine earlier decisions.
- Preserve all existing tracked and untracked work. Never clean, reset, stash,
  or discard a dirty worktree to satisfy a build precondition.

## Rust, formatting, and verification

- Use the exact Rust toolchain selected by `rust-toolchain.toml`; do not invoke
  an unpinned global `rustfmt`, `cargo`, or `clippy`.
  Run the commands below from the repository with that toolchain selected.
- Before any mutating formatter command, run `make fmt-check`. If the committed
  baseline or untouched files fail, do not run `cargo fmt` or accept incidental
  repository-wide churn. Report the baseline problem and keep formatting work
  separate from behavioral changes.
- After the formatting baseline is clean, format only as an intentional step,
  inspect the complete diff, then rerun `make fmt-check`. Never undo formatter
  churn with a command that could discard another person's dirty changes.
- Run locked checks (`cargo test --locked --workspace` and the relevant locked
  Clippy command) so dependency resolution matches the committed lock file.
  `make test` covers default members and does not replace the workspace check.
  `make lint` runs `cargo clippy --locked --all-targets -- -D warnings` for
  default members; include affected non-default members, features, and targets
  when choosing the relevant Clippy command.
- Complete required checks, then choose additional verification for the changed
  behavior. Add tests that exercise meaningful behavior or regressions, not
  tests that merely reproduce the implementation. Broaden or repeat passing
  checks only for new changes, failures, or unresolved concerns.
- Use `make build` or `make dev` for runnable payloads. Their build wrapper
  stages the exact generated `espeak-ng-data` and license notices beside the
  executable; a direct `cargo build` is not a complete distributable payload.

## Release history

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

## Windows helpers and deployment

- Windows helper source and build targets are owned under `windows-helpers`;
  preserve their GPL-2.0-or-later notices and separate executable boundary.
- Final Windows deployment is owned by the sibling Emacsvox repository. Use
  `make windows-omnivox-main-dev` there when its guard accepts a main-server or
  main-only audio-output change and a verified development runtime is already
  staged. Use `make windows-omnivox-dev` when that guard rejects helper,
  protocol, dependency, toolchain, companion, or shared-code changes. Reserve
  `make windows-omnivox` for a clean reproducible release that rebuilds every
  payload.
- Passing the main-only path guard does not waive an ADR's deployment
  requirements: ADR 0010 requires full development staging for its public
  protocol changes.
