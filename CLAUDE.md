# Omnivox Contributor Context

Read [AGENTS.md](AGENTS.md) before working in this repository. It is the
authoritative workflow and safety contract. This file is a compact technical
index for tools that automatically look for `CLAUDE.md`; it deliberately does
not duplicate test counts, status lists, or phase-by-phase roadmaps.

## Canonical references

- [README.md](README.md) — project overview, integration choices, build entry
  points, and documentation index.
- [STATUS.md](docs/STATUS.md) — implemented behavior and current limitations.
- [ARCHITECTURE.md](docs/ARCHITECTURE.md) — runtime ownership, hot path, bounds,
  replacement, cancellation, and lifecycle invariants.
- [CONTROL-PROTOCOL.md](docs/protocols/CONTROL-PROTOCOL.md) — version 1 control, legacy
  framing, tracked completion, and marker dispatch.
- [PRESENTATION-TIMELINE-PROTOCOL.md](docs/protocols/PRESENTATION-TIMELINE-PROTOCOL.md) —
  structured timeline versions and multipart transport.
- [HELPER-PROTOCOL.md](docs/protocols/HELPER-PROTOCOL.md) — isolated engine process contract.
- [ENV-VARS.md](docs/ENV-VARS.md) — CLI, environment, Emacsvox, and upstream
  Emacspeak configuration boundaries.
- [NEXT_STEPS.md](NEXT_STEPS.md) — remaining work only.

When those documents disagree, inspect the code and update the relevant
canonical reference rather than adding another explanation here.

## Workspace map

```text
omnivox-core/          legacy commands, queue/state types, pure timeline model
omnivox-audio/         canonical PCM, effects, resources, renderer, playback
omnivox-tts/           engine contracts/backends, routing and wire protocols
omnivox-cli/           executable, admission, work queue and synthesis pipeline
omnivox-piper-helper/  optional isolated Piper executable
omnivox-piper-sys/     optional native Piper bridge
elisp/                 standalone upstream-Emacspeak adapter
```

Important implementation entry points:

- `omnivox-cli/src/server.rs` — bounded reader/protocol loop, work requests,
  hard generations, keyed cancellation, worker, and terminal reporting.
- `omnivox-cli/src/work_queue.rs` — bounded nonblocking admission, same-domain
  replacement, and protected ordered/urgent work.
- `omnivox-cli/src/pipeline.rs` — text chunks, routing/fallback, synthesis,
  timeline lowering, effects, and audio tickets.
- `omnivox-cli/src/text.rs` — punctuation, `[*]` compatibility separator,
  source-offset maps, CamelCase splitting, and sentence/clause-aware chunking.
- `omnivox-cli/src/engine_execution.rs` — uncancellable native-call isolation.
- `omnivox-cli/src/routing.rs` — dispatch-local route resolution and retry.
- `omnivox-tts/src/control.rs`, `timeline_protocol.rs`, and
  `helper_protocol.rs` — authoritative wire types and bounds.
- `omnivox-audio/src/output.rs` — stream and per-source cancellation, short
  speech fade, playback tickets, and frame cues.
- `omnivox-audio/src/timeline.rs` and `post_synthesis.rs` — bounded rendering
  and persistent effects/tails.

## Core invariants

- Preserve all tracked and untracked user work.
- Use the exact checked-in Rust toolchain and locked Cargo commands.
- Run the non-mutating format check before considering a mutating formatter.
- Protocol admission must remain bounded and responsive while synthesis runs.
- Atomic submissions never play a valid prefix after validation failure.
- Ordered and urgent timelines never wait in the replaceable coalescing window
  and are never evicted as replaceable work.
- Admission of a keyed replacement atomically cancels only its exact
  protocol/key domain; failed admission leaves the prior owner intact. Hard
  stop is the only stream-wide engine/audio cancellation boundary.
- Stale PCM, markers, semantic callbacks, and terminal ownership cannot escape
  after cancellation.
- Fallback may degrade optional capabilities but must not silently lose source
  text.
- Mixer-source consumption is not proof of first audible device output.

## Validation entry points

```sh
make fmt-check
cargo test --locked --workspace
make lint
```

Use `make build-piper` only when the optional native Piper dependencies and
model workflow are in scope. Do not install tools or dependencies merely to
run a diagnostic. Real audio, helper, and onset claims require explicit
platform/backend evidence in addition to unit tests.

For a local Windows runtime from active changes, run
`make windows-omnivox-dev` in the sibling Emacsvox repository. The clean
release target is `make windows-omnivox`; never weaken its worktree or
provenance checks.

## Integration boundary

Emacsvox owns the complete structured adapter and Windows staging. Upstream
Emacspeak uses this repository's smaller `elisp/omnivox-voices.el`, whose
`dtk-*` names are upstream API names. Do not mix those variable names with
Emacsvox's intentional `tts-*` namespace or bundled adapter.

The optional Eloquence and DECtalk helpers require user-supplied proprietary
runtimes. Never add those binaries to this repository or imply that Omnivox
distributes them.
