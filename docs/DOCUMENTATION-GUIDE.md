# Documentation responsibilities and workflow

Use documentation to make current behavior, accepted constraints, proposals and
evidence easy to distinguish. Each fact has one maintained home; other documents
summarize it briefly and link there. Git retains implementation history.

Use direct verbs for documentation and component responsibilities: a reference
specifies a contract, a guide explains setup, and a component provides a service.
Reserve ownership terms for specific technical relationships, such as resource
lifetimes, process supervision, exclusive access and file ownership.

## Choose the document

| Document | Required when | Contents |
| --- | --- | --- |
| [ADR](adr/README.md) | A durable architectural choice changes boundaries, ownership, compatibility, dependencies, distribution, or a significant quality tradeoff. | Context, decision, alternatives, consequences and related decisions. |
| [Architecture](ARCHITECTURE.md) | Implemented component responsibilities, data flow or lifecycle change. | A coherent account of the current system and its invariants, linked to rationale and exact contracts. |
| [Roadmap](ROADMAP.md) | Work is proposed, scoped, deferred or completed. | Outstanding work, remaining acceptance and links to active plans. |
| Plan / implementation specification in `plans/` | Nontrivial future work needs a concrete contract, sequence or acceptance checklist. | Proposed behavior, non-goals, open decisions, compatibility, delivery slices and verification criteria. |
| Protocol or persisted-format contract | Messages, serialized fields, defaults, limits, ordering or compatibility change. | Exact normative shapes and semantics, negotiation, examples and version rules. |
| Operations/reference guide | Users or operators must install, configure, diagnose or use a feature. | Current supported commands, prerequisites, behavior and limitations. |
| [Status](STATUS.md) | Implementation or platform/runtime qualification changes. | Current capability and acceptance coverage, distinct from release publication. |
| [Evidence report](benchmarks/README.md) | A test, benchmark or experiment supports a compatibility, reliability, performance or platform claim. | Observed result, method, provenance, raw artifacts and limits. |
| Changelog / release notes | An implemented change is prepared or published. | User-visible changes and immutable published history under the release workflow. |

An ordinary bug fix, refactor within existing boundaries, test addition or
implementation milestone needs no ADR unless it changes a significant decision.
An additional configuration field or schema increment does not by itself justify
a separate ADR; keep exact fields and compatibility rules in their reference
when the existing decision already explains the policy.
New engine adapters need a separate ADR only when existing process, runtime and
distribution policies do not settle their choices. A plan is not required for a
small straightforward change. Avoid creating a document for each commit.

## Locations and naming

Keep the root index, architecture, status, roadmap, licensing and this guide in
`docs/`. Use lowercase descriptive names in subject directories:

- `engines/`: engine setup, runtime supply and engine-specific verification.
- `guides/`: installation, configuration, operation and diagnostic procedures.
- `reference/`: internal algorithms and persisted-format contracts.
- `protocols/`: exact wire contracts, including cross-protocol tuning rules.
- `plans/`: active proposals and nontrivial future implementation specifications.
- `adr/`: numbered durable architectural choices.

The [documentation index](README.md) is the common navigation entry; avoid an
extra index or document when a short section in an existing document suffices.
Engine guides link shared configuration and managed-voice procedures instead of
copying them. The Emacsvox repository documents its client UI and settings;
this repository documents the upstream Emacspeak compatibility adapter.

Protocol examples under `protocol-fixtures/` are executable test inputs; keep
those paths stable unless the test integration is deliberately changed. Each
wire or storage contract has one authoritative home. A move does not create a
second schema, change the format version or promote a proposal to implementation.

Keep existing `benchmarks/`, `experiments/` and `rate-audits/` evidence packs
intact. The [evidence index](benchmarks/README.md) groups performance baselines,
functional acceptance and investigations without rearranging raw inputs.

## ADR lifecycle

Keep one coherent architectural subject per record, normally a short document.
Use a descriptive numbered filename and these sections:

```text
# ADR NNNN: Decision title
- Status: Proposed | Accepted | Rejected | Superseded
- Date: decision/proposal date
- Related: applicable decisions

## Context
## Decision                 (Proposed decision while under discussion)
## Consequences and alternatives
```

Before creating any new record, including a proposed ADR, obtain explicit
approval to create it. Explain the durable choice, why the existing decisions or
references are insufficient, and the proposed scope. Approval to implement a
feature does not imply approval to create an ADR; approval to create a record
does not itself accept its decision. If creation is not approved, continue work
covered by existing decisions and keep any unresolved architectural choice open.

An approved record should explain a durable constraint that would otherwise
require a future contributor to guess its rationale. Describe what changes, why
this option was chosen and what costs it creates. Link detailed algorithms,
schema tables, commands and test evidence instead of copying them into the decision.

Drafts may be edited freely while proposed. Record acceptance only after explicit
approval of the decision or authorization to implement its defined scope.
A documentation rewrite, passing test or completed parser does
not itself accept a proposal. Rejection records its reason. Acceptance and
implementation/release status remain separate.

After acceptance, preserve the decision and rationale. Correct spelling and
links or clarify wording without changing meaning. A material change uses a new
ADR that identifies the exact record or sections extended/superseded; link both
ways and update the index. Do not silently treat a partial refinement as repeal
of the whole older policy. Keep IDs stable and do not reuse numbers.

The approved consolidations on 2026-09-27 and 2026-09-28 are explicit exceptions.
The first replaced the earlier collection; the second combined the local
configuration decisions and renumbered Windows output recovery. Both preserve
accepted constraints and evidence. The [ADR index](adr/README.md) records the
numbering changes and historical revisions. These exceptions do not authorize
further consolidation or renumbering without explicit approval.

## Plans and specifications

Start a plan with its status, scope and related roadmap/ADR links. Distinguish
proposed, accepted for implementation, in progress, completed and withdrawn.
Identify open choices explicitly; do not hide unresolved decisions behind
examples or accept future fields with ignored behavior.

Use enough detail to make implementation reviewable: configuration/format
semantics, defaults, precedence, bounds, failure behavior, compatibility and
observable acceptance criteria. Keep a short checklist or current status; do
not append an implementation diary, test-count chronology or obsolete next steps.
Routine choices within accepted scope can be resolved during implementation;
pause for a material policy conflict or unapproved boundary change.

When implementation completes, reconcile the architecture, user guides, protocols
and status with the actual code. Move enduring specification content into its
maintained reference, preserve test evidence, update the roadmap, and remove the
obsolete plan. A withdrawal retains its reason in the roadmap or decision when
useful. Git holds abandoned implementation sequences; parallel historical plans
and redirect files are not normally needed. Existing historical references can
be retired incrementally with the same evidence-preservation check.

## Protocols and compatibility

Protocol references describe implemented contracts. Proposed extensions stay
labelled in a plan until their shapes and compatibility are accepted. Acceptance
may precede implementation, but a reference must clearly label an unimplemented
extension and must not claim its capability is advertised.

Specify envelope/version/capability rules, required and optional fields, omission
versus empty/default, decoded and encoded bounds, ordering, correlation,
cancellation, errors and old-peer behavior. Define persisted configuration with
the same precision. Wire types, fixtures and prose must agree; changing a fixture
alone cannot authorize a semantic change. Public compatibility changes need an
ADR when the existing extension policy does not settle the decision.

## Evidence and performance

Preserve raw reports, measurements, fixtures, reproduction tools and provenance.
New runs receive new identities; they do not overwrite old samples. Editorial
link repairs may update report navigation without altering results or their
historical qualification. Record a substantive correction explicitly with its
reason, retaining the original observations.

Before discarding a diary or plan, extract unique results into a compact dated
evidence report and link existing raw artifacts. Mark missing raw data, commands,
versions and sample counts rather than manufacturing provenance. Historical pass
counts, simulated tests, native PCM, source-consumption timing and acoustic
acceptance support different claims.

Follow the [baseline comparison procedure](benchmarks/README.md#comparing-a-future-candidate).
Select affected workloads and criteria before measuring, preserve matched build
conditions, and report uncertainty and missing coverage. A performance change
needs relevant before/after evidence; documentation-only work does not require
rerunning native benchmarks or making new performance claims.

## Reading, editing and checking

Start with the architecture reference and ADR index. Read the applicable accepted
records and their dependencies; widen to all accepted decisions for cross-cutting
or uncertain scope. Proposed documents cannot override accepted constraints.
Check actual code and runtime qualification before converting old progress prose
into a claim of current behavior.

Update incoming links when moving/removing material, including prose ADR numbers,
Org references, repository instructions and paired Emacsvox references. Keep the
README as navigation, the roadmap as the entry to future work, and the ADR index
as the entry to decisions. Avoid maintaining duplicate lists in each.

Run `make docs-check` and `git diff --check` before review/commit. Stage new
documents before checking tracked sources. The link checker covers Markdown and
Org targets and anchors, and current-branch GitHub links to this repository.
For paired Emacsvox references, use `make docs-check-paired
EMACSVOX_SOURCE_DIRECTORY=/path/to/emacsvox`; it checks incoming links without
network access or changing either checkout. Commit/tag-pinned historical URLs
remain historical. General external availability needs a separate network audit.

Run affected fixture/code tests when executable contracts change. Follow the
relevant repository's additional documentation gates for cross-repository edits.
Never change a published changelog section or a retained measurement merely to
make a documentation check pass.
