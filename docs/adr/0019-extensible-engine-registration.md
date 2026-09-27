# ADR 0019: Extensible engine registration

- Status: Proposed
- Date: 2026-09-27
- Extends: [ADR 0001](0001-speech-engine-process-boundaries.md) with explicit
  local registration; preserves its process and runtime-distribution boundaries.

## Context

The helper protocol is engine-neutral, but Omnivox's launch paths enumerate
known helper IDs, executable names and environment variables. An independently
maintained adapter therefore needs a server change or another engine's identity
to join the registry. Reusing an ID misidentifies voices and conflicts with the
real engine. Separate startup and diagnostic launch definitions also create
opportunities for inconsistent behavior.

ADR 0001 already permits Omnivox-maintained helpers that load user-supplied
runtimes. Adapter maintenance, helper distribution and runtime distribution are
separate choices. A registration framework should support those helpers and
independent adapters through the same contract, preserving identities when
maintenance or distribution arrangements change.

## Proposed decision

### Share registration and lifecycle

Normalize compiled in-process factories, shipped helper definitions and explicit
external registrations into one registry. Startup, inventory, exact diagnostics,
selection and recovery consume the same resolved definitions. Existing native
factories keep their process boundaries; external registration introduces only
helper processes using the existing negotiated protocol.

Keep registration, live descriptors and routing policy distinct. Registration
defines launch inputs; descriptors establish actual voices and capabilities;
policy determines eligibility. Configuration cannot claim successful native
validation or invent capabilities. Registered IDs must agree with descriptors;
external registrations cannot shadow shipped IDs or each other.

All helpers retain the common timeouts, cancellation watchdog, circuit breaker,
cleanup and recovery rules. Missing or failed optional helpers leave eligible
fallback speech available under [ADR 0017](0017-managed-engine-startup-fallback.md).
Exact operations report the selected target's failure. Malformed main policy
fails startup rather than silently removing exclusions.

### Use explicit local, versioned configuration

Use one JSON manifest per external helper in `helpers.d/` and a separate
`config.json` for local routing and launch overrides. Strict versioned readers
enforce bounded inputs, deterministic conflicts and field-specific precedence.
Existing CLI/environment overrides and shipped defaults retain compatibility.

The speech host owns this configuration. External programs require fully
absolute native paths and literal argument vectors. Working-directory or PATH
searches, shell expansion, network discovery and executable definitions supplied
through speech protocols are excluded. Registration authorizes launching local
code; process isolation does not provide an operating-system sandbox.

Move deployment choices such as helper paths, arguments, bounded operational
timeouts and preferences into configuration. Keep native ABI handling, calibrated
controls, protocol validation, hard limits and provider verification in their
existing implementations.

### Separate registration from automatic selection

External helpers default to explicit voice/engine selection or policy references
that name them. Registration alone cannot change default speech or make a voice
eligible for unrestricted property matching. Local automatic-selection opt-in
is separate from startup preference. Preserve shipped-engine eligibility and
require local disablement and voice-library exclusions to survive session-policy
replacement, previews and recovery.

### Activate immutable configuration at restart

Resolve configuration before engine construction and retain it for recovery.
File edits take effect at a deliberate new activation, not during an utterance
or helper replacement. Both speech workers receive the same prepared launch
snapshot while retaining independent native state and cancellation. Reuse
[ADR 0012](0012-voice-library-and-model-lifecycle.md)'s existing local activation,
ownership and rollback rules and
[ADR 0008](0008-remote-workstation-service.md)'s speech-host boundary.

Registration does not enroll an engine in managed acquisition, native download
validation or release packaging. Those integrations remain explicit provider
and distribution work. A maintained helper can load a user-supplied runtime
without including that runtime in Omnivox artifacts, under ADR 0001 and the
applicable component policy.

## Consequences

Independent engines can join Omnivox without a server rebuild. Maintained and
external adapters share lifecycle coverage, and stable engine/voice identities
can survive a change in distribution. Users without configuration retain
existing behavior.

Configuration becomes a compatibility contract requiring strict parsing,
diagnostics, bounded initialization and migration rules. Selection must enforce
external-engine eligibility at every route stage, rather than relying on list
order. Worker coordination must retain actual launch snapshots; an identifier
alone cannot prove both workers used the same configuration.

## Alternatives considered

- **Continue adding every helper to hard-coded lists:** keeps a small startup
  surface but requires server releases for independent integrations and retains
  duplicated launch definitions.
- **Use a delimited environment variable for all additional helpers:** makes a
  minimal prototype easy, but quoting, paths, arguments and per-helper settings
  become difficult to extend. JSON supports structured validation and editing.
- **Discover executable plug-ins automatically:** reduces setup but allows an
  installation or search-path change to introduce executable code implicitly.
- **Load external libraries into the main process:** conflicts with ADR 0001's
  isolation and architecture requirements; no new binary plug-in ABI is needed.
- **Make every registered engine an automatic candidate:** lets an added voice
  change existing speech without an explicit selection preference.

## Specification and delivery

The [framework specification](../plans/EXTENSIBLE-ENGINE-FRAMEWORK.md) owns the
exact schemas, paths, defaults, precedence, bounds and acceptance checklist.
The [roadmap](../plans/NEXT_STEPS.md#extensible-engine-registration) owns delivery
scope and status. Keep detailed configuration values in the specification so
this record does not become a second, divergent schema reference.

The first implementation proves registration and lifecycle with a fake helper;
individual native adapters still need platform/runtime acceptance. New language
matching, per-span language context, live reload and a configuration UI are
separate increments. This proposed record introduces no runtime behavior and
does not mark the detailed specification or a new native integration as accepted.
