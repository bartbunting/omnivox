# Omnivox Documentation

The repository [README](../README.md) is the project entry point. This index
separates maintained behavior, wire contracts, operations, future plans, and
historical material so a proposal cannot be mistaken for a shipped feature.

## Current behavior and design

- [STATUS.md](STATUS.md) — implemented features, limitations, platform support,
  and validation boundaries.
- [ARCHITECTURE.md](ARCHITECTURE.md) — runtime ownership, admission, routing,
  cancellation, audio, and failure handling.
- [ENV-VARS.md](ENV-VARS.md) — command-line, environment, Emacsvox, and upstream
  Emacspeak configuration.
- [TEXT-CHUNKING.md](TEXT-CHUNKING.md) — maintained chunking behavior and source
  offset rules.
- [ENGINE-ISOLATION.md](ENGINE-ISOLATION.md) — containment for uncancellable
  native synthesis.
- [LICENSING.md](LICENSING.md) — component boundaries and binary-distribution
  licensing. The root [LICENSE](../LICENSE) contains the MIT text for
  Omnivox-authored source.

## Protocol specifications

- [Legacy line protocol](protocols/LEGACY-PROTOCOL.md) — baseline Emacspeak
  command grammar, state, queueing, and limits.
- [Control protocol](protocols/CONTROL-PROTOCOL.md) — discovery, inventory,
  routing policy, preview, tracked completion, and marker dispatch.
- [Presentation timeline](protocols/PRESENTATION-TIMELINE-PROTOCOL.md) —
  structured Aural timelines, multipart transport, actions, and degradation.
- [Engine helper protocol](protocols/HELPER-PROTOCOL.md) — isolated synthesis
  engine process contract.
- [Validated fixtures](protocol-fixtures/) — JSON and JSONL examples checked
  against the public Rust wire types by `omnivox-tts` tests.

## Operations and releases

- [PIPER.md](PIPER.md) — optional companion build, layout, model setup,
  verification, upgrade, and removal.
- [DIAGNOSTICS.md](DIAGNOSTICS.md) — log collection, privacy boundaries, and
  optional Windows crash dumps.
- [Release and deployment guide](../.github/DEPLOYMENT.md) — archives,
  verification, installation, and physical acceptance checks.
- [Workflow reference](../.github/workflows/README.md) — CI and release job
  behavior.
- [Developer tools](../tools/README.md) — build staging, archive verification,
  diagnostics, stress tools, and manual audio tests.
- [CHANGELOG.md](../CHANGELOG.md) — published and unreleased user-visible
  changes.

## Plans and historical material

- [NEXT_STEPS.md](plans/NEXT_STEPS.md) is the active roadmap. Its entries are
  not promises of current behavior.
- [PIPER-RELEASE.md](plans/PIPER-RELEASE.md) records the audited gap between
  the experimental Piper helper and a reproducible cross-platform companion
  release, including the source-acquisition decision required before work.
- [SPEECHD-PLAN.md](plans/SPEECHD-PLAN.md) is an unimplemented design proposal
  that must be reconciled with current engine contracts before use.
- [CHUNKING-IMPLEMENTATION.md](history/CHUNKING-IMPLEMENTATION.md) is a short
  redirect from an obsolete implementation report to the maintained chunking
  reference. Git history holds other retired phase plans.

## Maintenance rule

Describe shipped behavior in the current references or protocol
specifications, future work in `plans/`, and obsolete context in `history/`.
Keep testable examples in `protocol-fixtures/` and update their Rust validation
when a wire contract changes. Run `make docs-check` after moving or linking a
document; CI applies the same local-link check to every tracked Markdown file.
