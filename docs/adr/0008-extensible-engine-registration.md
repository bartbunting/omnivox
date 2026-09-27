# ADR 0008: Extensible engine registration

- Status: Proposed
- Revised: 2026-09-27 during documentation consolidation; not accepted or implemented.
- Extends: [Engine isolation](0001-engine-isolation-and-distribution.md).
- Related: [Local activation](0007-managed-voice-lifecycle.md),
  [speech-host boundary](0004-workstation-service-and-worker-ownership.md).

## Context

The helper protocol is engine-neutral, but known IDs, program paths and launch
settings are enumerated in Omnivox. Independent adapters need a server change
or another engine's identity, which misidentifies voices and conflicts with the
real adapter. Separate startup and diagnostic definitions risk disagreement.

Maintained helpers already load user-supplied runtimes under ADR 0001. Adapter
maintenance, helper distribution and runtime supply should remain independent
choices, sharing a protocol and preserving engine/voice identity when maintenance
or distribution changes.

## Proposed decision

Normalize compiled in-process factories, shipped helper definitions and explicit
external registrations into one registry consumed by startup, inventory, exact
diagnostics, selection and recovery. Existing native factories retain their
boundaries. External registration introduces helpers, not an in-process binary
plug-in ABI.

Keep launch registration, live descriptors and routing policy separate. Strict
bounded versioned JSON defines one manifest per helper in `helpers.d/` and local
policy/overrides in `config.json`. Preserve existing CLI/environment precedence
and shipped defaults. IDs must match descriptors and cannot shadow shipped IDs
or other external registrations. Configuration cannot invent runtime validation,
voices or capabilities.

The speech host handles registration. External programs require fully absolute
native paths and literal argument vectors. Exclude working-directory/PATH
discovery, shell expansion, network discovery and executable definitions supplied
through speech protocols. Registering a helper authorizes local code to run;
process isolation is not an operating-system sandbox.

Move deployment choices such as program, arguments, bounded operational timeouts
and preferences into data. Native ABI handling, calibrated mappings, hard protocol
limits and provider verification remain in their maintained implementations.
All helpers share watchdog, circuit, cleanup and recovery behavior. Optional
failure preserves eligible ordinary speech; exact operations report their target's
failure and malformed main policy cannot silently remove exclusions.

External engines default to explicit selectors or policy references naming them.
Registration alone cannot change default speech or unrestricted property matching.
Local automatic-selection permission is distinct from startup preference; local
disablement and managed exclusions survive session replacement and recovery.

Resolve an immutable launch snapshot before construction and retain it for helper
recovery. File edits take effect at explicit restart/activation. Both workers
receive the same prepared configuration with independent native state; use the
existing local owner, retirement and rollback contract. Registration does not
enroll a helper in acquisition, native download validation or release packaging.
A maintained adapter may load separately supplied proprietary libraries without
redistributing them under the existing component policy.

## Consequences and alternatives

Independent adapters can join without rebuilding the server, with the same
lifecycle checks and stable identities as maintained helpers. This introduces
a configuration compatibility contract, bounded initialization and eligibility
checks at every routing stage. A shared identifier alone cannot prove two workers
received the same immutable configuration.

Keeping hard-coded lists requires server releases for independent adapters.
A delimited environment list scales poorly to paths, arguments and per-helper
settings. Automatic discovery introduces executable code implicitly. In-process
external loading conflicts with isolation; unrestricted automatic selection lets
an added voice change existing speech without an explicit preference.

The [specification](../plans/extensible-engine-framework.md) defines exact schemas,
precedence, bounds and acceptance criteria. The
[roadmap](../ROADMAP.md#extensible-engine-registration) tracks remaining work.
The first slice uses a fake helper; real native integrations retain separate
qualification. Language-routing changes, live reload and a configuration UI are
later increments. This proposal does not authorize their implementation or change
current runtime/distribution behavior.
