# Engine voice parameters: wire and compatibility contract

Accepted implementation boundary, 2026-09-17. This mirrors the semantic design
in Emacsvox commit `e0e83d71d`, under [ADR 0015](adr/0015-engine-described-voice-parameters.md).
The [JSON fixtures](protocol-fixtures/engine-voice-parameters.json) are independent
examples for codecs and composition. The typed metadata and pure planner now
consume the parameter and composition examples. Public catalogue discovery is
implemented; native editing/speech operations below remain reserved. The
[native audit](benchmarks/2026-09-17-native-voice-parameters.md) does not replace
the remaining execution and cancellation tests.

## Implementation status

`omnivox-tts::native_parameters` implements bounded typed catalogues and sparse
native blocks, runtime/voice identity checks, range and scope validation, value
provenance, contextual masking and deterministic side-effect ordering. Its JSON
readers reject duplicate keys and unknown fields. Unknown schemas can survive
an inert save/load round trip but cannot execute against a different catalogue.
The generic planner rejects dependency cycles and side effects whose target has
no restorable value; an adapter-specific execution plan is still required for
profiles with those dependencies.

Adapters supply their already composed and mapped common values. The independent
fixtures include those inputs and expected results, preserving the existing
mapping formulas rather than introducing a second calibration here. Unit tests
cover equal-value context (including zero rate offset), omission/default/zero,
unknown default values, separate choices for the same physical voice, stale
runtime evidence and all-or-nothing native validation. They exercise planning;
they do not establish native reset, actual synthesis fallback or cancellation.

The Windows helpers now contain optional ECI voice APIs and DECtalk parameter
readback. The [binding audit](benchmarks/2026-09-17-native-parameter-bindings.md)
checks their endpoints, ordinary resets, unit-mode guards and missing bindings
against the installed runtimes. Both Windows adapters now connect their
qualified native controls to helper 6.
The pre-existing Windows cancellation ordering race was fixed and qualified in
[the cancellation follow-up](benchmarks/2026-09-17-windows-helper-cancellation.md).

Eloquence now has an internal native execution path for its qualified Windows
ECI 6.1 American English runtime. It validates and freezes sparse integer/default
edits, composes them with existing common mappings and explicit context, applies
on its STA owner thread, verifies readback before PCM, and restores the pristine
preset on success, cancellation and failure. The
[execution audit](benchmarks/2026-09-18-eloquence-native-execution.md) exercises the
actual helper bytes. Its helper-6 catalogue, explanations and application
receipts are now connected; public transport remains pending.

DECtalk now also has an internal execution path for its 28 qualified design
voice controls. It synchronizes command-only preparation, verifies readback
before PCM, and coordinates native cancellation with pristine restoration.
Its [execution report](benchmarks/2026-09-18-dectalk-native-execution.md) records
the accepted runtime and verification scope. Ordinary common mappings remain
unchanged, including clamping performed by DECtalk itself. Its
[helper-6 handler](benchmarks/2026-09-18-dectalk-helper6.md) now exposes all 28
controls, planned explanations that account for those clamps, and verified
applied receipts. Catalogue queries do not touch the active native voice.

The legacy helper wire boundary now rejects duplicate members and unknown
request/synthesis fields instead of silently discarding future native controls.
The Windows host checks strict JSON before dictionary parsing. Existing versions,
valid request shapes and unowned-error compatibility remain intact. The
[wire-boundary report](benchmarks/2026-09-18-helper-native-wire-boundary.md)
records the regression and compatibility checks for versions 1–5.

`helper_protocol::parameters` now provides the helper-6 parameter
request/response codecs, correlated application evidence and atomic catalogue
page assembly. It reuses the legacy duplicate-free envelope reader and common
synthesis validation, while keeping the negotiated 1–5 readers unchanged.
The [codec report](benchmarks/2026-09-18-helper6-parameter-codecs.md) records its
scope. Both Windows helpers now execute these messages when 6 is explicitly
negotiated, and the Rust parent now offers 6 before the unchanged older versions. The
[handler report](benchmarks/2026-09-18-eloquence-helper6.md) records direct wire
and native acceptance. Catalogue reads use immutable metadata and unknown
preset defaults, avoiding owner-thread waits or voice changes. Verified native
readback is retained only as applied-plan evidence.

The Rust planner and native request path are not connected to public speech
operations yet. Common mappings and capability advertisements remain unchanged.
The [parent integration](benchmarks/2026-09-18-helper6-parent.md) connects strict
negotiation, ordinary and native synthesis, per-request application evidence,
and bounded catalogue/explanation queries. Queries return busy during speech;
they never connect a deferred helper or restart one to obtain metadata.
Public [read-only catalogue discovery](benchmarks/2026-09-18-public-parameter-catalogues.md)
now reaches those current-worker queries
through the `engine_parameter_catalogue_v1` capability. Admission and replies are
bounded independently of the speech command thread. Native registration, speech,
preview/explanation operations, routed receipts and the Emacs editor remain next.
Adapter integration must preserve the old common path, including its existing clamps; native edit ranges are separately
qualified and must not silently recalibrate common controls.

Internal [native choice admission and preparation](benchmarks/2026-09-18-native-choice-admission.md)
now stores engine-layered definitions in the existing registry generation domain.
It validates complete replacements against immutable current metadata, preserves
unavailable settings, and prepares the actual selected choice with its common
context and runtime identity. Registration performs no engine I/O. Older speech
paths reject these definitions instead of discarding native settings. Public v3
registration, routed execution/evidence, previews and editing remain pending;
the native execution bundle is not advertised.

## Shared rules

Retain control envelope 1, remote envelope 1, positive request correlation,
256 KiB decoded control messages, bounded multipart transport and one logical
registry generation domain. Reject duplicate keys, unknown fields, mixed tagged
forms, nonfinite values and invalid IDs before mutation. Identifiers are 1–128
ASCII characters from `[A-Za-z0-9_.-]`. Do not intern remote IDs as Lisp symbols.
Use scalar native units, not normalized ACSS or raw vendor strings.

New success types are listed below. Malformed requests use the existing control
`error` shape with `malformed_request`; structurally valid invalid values use
`invalid_configuration`; conflicting expected identities use `stale_generation`.
Payload size and unsupported operation/version errors retain existing codes.
Operationally busy or unavailable read-only queries return the typed result
below, not a synthesis failure. Every query terminates within five seconds;
pending UI state is client-owned, not an unbounded server request.

These examples use synthetic revisions/generations. A real catalogue revision
is a lowercase SHA-256 digest of deterministic descriptor content, excluding
connection identity. `profile_id` identifies qualified implementation behavior;
`schema_id` identifies parameter semantics/units. Runtime generation is a
positive u64 assigned within the connection for a helper/runtime incarnation.
An engine unavailable before initialization has no runtime identity.

## Catalogue

Capability `engine_parameter_catalogue_v1` introduces:

- Request `get_engine_parameters_v1`: `engine_id`, required nullable `voice_id`,
  required nullable `cursor`, required nullable `expected_catalogue_revision`.
- Response `engine_parameters_v1`: `engine_id`, `result`.

The result is discriminated by `status`:

- `ready`: exactly `identity`, nullable `voice_id`, `parameters`, `mappings`,
  `next_cursor`. `next_cursor` is null at the end; otherwise an opaque bounded
  ASCII token up to 128 bytes. Every page repeats the same identity/mappings.
- `busy`: exactly `retry_after_ms` (integer 1–5000).
- `unavailable`: exactly `reason`, `message`. Reasons are `engine_unavailable`,
  `voice_unavailable`, `not_described`, or `unsupported_helper`.

Identity contains `schema_id`, `profile_id`, `catalogue_revision`, and
`runtime_generation`. Subsequent pages require both the cursor and expected
revision from the preceding page. First requests may specify an expected
revision as a conditional guard. A stale cursor/revision fails; clients do not
merge pages across revisions. Page at 64 parameters, no more than 512 per
engine, also respecting byte limits. A single oversized descriptor is an
error, not silent truncation. Refresh never runs a managed-installation checker.

With `voice_id: null`, report engine-level metadata without claiming a voice's
defaults. A specific voice query may use cached readback or return busy until
its native owner thread is available. It must not stop speech or load every
installed model. Profiles may supply evidence without instantiating a model.

A parameter has exactly these fields:

- `id`, `label` (at most 128 UTF-8 bytes), `help` (at most 1024 bytes),
  `group` (identifier), `order` (u32), `unit` (nullable identifier).
- `value_type`: tagged `kind`. Integer and number kinds require `minimum`,
  `maximum`, `step`; booleans have no additional fields; enum requires
  `choices`, 1–64 unique `{value, label}` records with identifier values.
  Integer constraints are integral; numbers are finite. Step is a positive
  UI increment, not an implied grid constraint.
- `scope`: `voice`, `engine`, or `startup`; `adjustable`: boolean.
- `availability`: `{status, reason}`. Status is `supported`,
  `voice_unavailable`, `runtime_unsupported`, or `not_checked`. Reason is null
  for supported and otherwise a short nonempty string.
- `default`: `{source, value, reset_supported}`. Source is `runtime_readback`,
  `qualified_profile`, or `unknown`; value is a typed scalar or null. Unknown
  requires null; the other sources require a typed value. Null does not preclude
  a verified reset with unknown number.
- `side_effects`: list of other parameter IDs that an application can modify.

Only supported, adjustable, voice-scoped descriptors accept palette operations.
An adjustable voice descriptor also requires verified reset support, even when
the user only sets explicit values; omission must never inherit a prior request.
An unknown descriptor kind can be retained as bounded inert client data, but is
never executable. An unknown native operation shape is rejected. Profile code,
not remote metadata, owns actual native calls and dependency planning.

`mappings` contains `{common_inputs, native_outputs}` records. Inputs are the
existing common dimensions, including `rate`/`rate_offset` and volume when the
mapping uses them. Outputs reference catalogue parameter IDs. Mapping tables
are not formulas to evaluate in Emacs. Side effects/order are adapter concerns;
the client displays their consequences. A profile cannot advertise independent
control where its application plan cannot honor it.

## Native records and storage projection

A native record has exactly `engine_id`, `schema_id`, `parameters`. Parameters
is a map of at most 64 IDs to `{op:"set",value:SCALAR}` or `{op:"default"}`.
Default forbids value; omission inherits. Zero and false are values, not resets.
`set` strings are only legal for validated enum values. Require an explicitly
engine-constrained selector matching `engine_id`; no physical voice change is
encoded as a parameter. The engine's catalogue is authoritative for execution.

Version-4 stored choice records add optional `:native`; use string parameter IDs
and explicit operations. The new wire choice form requires `native`, which is
null when no record exists. Existing `id`, `selector`, and common `adjustments`
retain their types/meaning. The fixture's storage cases show portable and local
projection; storage units remain native for this field, while common Lisp
patches continue using the existing raw editor units.

Emacsvox allocates palette/routing-file/choice-set schema 4 and user-data
envelope 10. Routing-profile schema remains 2. Readers/writers must preserve
inert unknown native schemas/IDs before enabling the new writer. A portable
engine-constrained selector retains its native record; exact local choices
remain local. Publish immutable local snapshots before palette references and
then acknowledge main/notification application separately. No opening/preview
operation upgrades storage or writes a setting.

## Registry, timelines and previews

Capability `engine_voice_parameters_v1` requires the catalogue capability,
`presentation_timeline_v5`, and `playback_marker_events_v4`. Advertise it only
after every normal named-voice speech path and private preview implements it.
The server bundle does not assert that every attached helper has native controls.

`register_logical_voices_v3` retains v2's registry fields and adds definition
mode `engine_layered`. Its definition is the existing layered shape with the
extended choice records. Legacy and ordinary layered modes remain unchanged.
Reject native members in old definition kinds. Acknowledgement
`logical_voices_registered_v3` retains v2 fields and adds `native_status`, one
record per non-null native block: `{logical_voice_id, choice_id, status, reason}`.
Status is `supported`, `deferred`, or `unavailable`; reason is null only for
supported. Default/property selectors may need per-actual-voice validation at
synthesis, even when their schema/profile is already supported.

Known invalid settings reject registration before changing the registry. A
well-formed block for an absent engine/schema remains registered but unavailable;
it cannot be sent as raw settings to an unqualified helper. Ordinary speech
uses that choice's existing common tuning, with native degradation, when the
entire native block is inapplicable. No silent partial application of a block.

Timeline 5 keeps v4 envelope/action semantics and adds span mode `engine_layered`
with the same span fields as layered mode. Require the reference kind to match
the registered definition. Reject references from an older timeline to a new
definition kind. Freeze registry/style/context/rate/policy at admission. Preserve
versioned replacement domains and no replay after reconnect.

`preview_voice_v3` retains all v2 fields and selection semantics; its private
voice uses extended choices. It always requires faithful native execution.
An inapplicable block rejects the preview before audio, rather than dropping
settings. Automatic synthesis fallback still works before PCM commitment;
selected-choice preview never substitutes another selector. Preflight both
halves of a comparison. Preview neither replaces the registry nor activates
downloaded voices.

`preview_voice_completed_v3` retains v2 terminal fields; each accepted-audio
record and non-null last-started record adds `native_application`, described
below. A separate common-only audition uses a deliberately projected old
request and is labelled accordingly; it is never a successful native preview.

## Composition and helper 6

Resolve the actual choice and compose common shared/choice/context layers as
before. Preserve the explicit contextual operation IDs even when their values
equal the base. Build a fresh native plan from the selected pristine voice,
mapping the final common settings once. Overlay native operations except where
an explicit contextual mapping writes that native output. An output depending
on multiple common inputs is masked when any explicit context input drives it.
Never infer a common slider position from native values or remap after overlay.

The independent composition examples supply expected native values. A native
default selects a pristine voice baseline, not midpoint/zero/last-used state.
Application side effects must not overwrite a contextual winner: reassert its
value in adapter order, or reject an unrepresentable combination. Reset every
independently styled unit. Fallback recomposes from frozen inputs for its actual
choice, never a failed attempt's plan. Output/PCM errors retain the existing
rules, including no retry after the first accepted PCM.

Helper protocol 6 retains v5 framing, bounded streaming and anchors. Its
`synthesize` request adds required `voice_parameters` (nullable). A non-null
record has `native`, `context_dimensions`, `expected_identity`, and
`unavailable_policy` (`require` for strict preview; `common_only` for ordinary
speech). Native is the same block above; expected identity includes schema,
profile, revision and runtime generation. Context dimensions are unique existing
patch field names. The actual voice still comes from settings. No raw native
command or executable plan crosses this boundary.

The helper validates identity, computes the native plan using its existing
common settings, applies it and reports compact `native_application` in the
v6 `synthesis_started` event before PCM. An invalid strict request fails before
audio. Common-only policy can withhold an inapplicable block with explicit
status; it cannot excuse malformed protocol data. Validation does not poison
engine health. Native call failures retain ordinary synthesis health behavior.

Helper 6 also adds `get_engine_parameters_v1` with the same data/result shapes
as the control operation, scoped to its engine and validated by the server.
Its `explain_voice_parameters_v1` uses a resolved draft source containing
`mode:"draft"`, `settings` and `voice_parameters`, or an applied source
containing `mode:"applied"` and `plan_id`. A ready helper explanation has the
control result fields except `choice_id`; the server supplies that identity
from its frozen choice. Helpers do not resolve a palette's fallback chain.
Actual native access stays on the helper owner thread. Static planning can run
from immutable metadata; a busy native query returns busy. Helpers 1–5 keep
their original request shapes. Optional new exports do not become requirements
for old speech.

## Explanation and receipts

`explain_voice_parameters_v1` has required `source`, discriminated by `mode`:

- `draft`: `voice`, `context`, `placement`, `selection`, `fallback_policy`,
  `disabled_engine_ids`, and nullable `expected_base_rate`. These match preview
  v3 except no text is accepted and selection must name one choice. Resolution
  is a prediction without text/repertoire admission, not proof of playback.
- `applied`: `plan_id`, scoped to this connection's bounded recent-plan cache.

Response `voice_parameters_explained_v1` contains `result`. Ready result has
exactly `status`, `evidence` (`planned` or `adapter_applied`), nullable `plan_id`,
`choice_id`, `realized`, `identity`, `parameters`. Each parameter has `id`,
nullable typed `value`, `origin` (`common_mapping`, `context_mapping`,
`native_set`, `native_default`, `engine_default`), `masked_native`, and
`read_back` (boolean). A planned result never claims readback of applied values.
An applied plan can report acknowledged setters without claiming numeric
readback. Busy uses the catalogue busy shape; unavailable uses `{status,reason,
message}` with reasons including `plan_expired`, `native_unavailable` and
`voice_unavailable`. A lookup never synthesizes to recreate expired evidence.

An adapter retains at most 64 recent plans and 256 KiB total detail for its
own worker, evicting oldest first. The server retains at most 64 plan references
per connection and maps helper-local plan IDs to connection-scoped opaque IDs;
it need not duplicate the helper's detail cache. At most 64 requested/native-
related parameter rows per plan; catalogue browsing remains separately paged.
Worker replacement invalidates those references. Output evidence after eviction
still retains compact identity/status but detail lookup reports unavailable.

`native_application` is null without a requested native block; otherwise exactly
`status` (`applied` or `common_only`), nullable `plan_id`, nullable `identity`,
`masked_parameters` (bounded requested IDs), and nullable `reason`. Applied
requires identity, plan ID and null reason. Common-only requires a reason and
cannot claim applied native values. For marker events 4, the existing voice
choice receipt gains this member. Keep committed-PCM and actual-start receipts
distinct and retain their existing dispatch/choice identity.

## Required acceptance

Use the fixtures for codec round trips, explicit equal-value context, engine
defaults, unknown schemas, native zero/false, same-voice different choices and
old-target projection. Add duplicate-key/raw invalid cases before enabling new
readers. Old operation codecs must reject new members rather than ignore them.
Keep the 32-choice and 16 KiB preview text bounds, count native bytes in queues,
and reject overlarge descriptors, enum lists, plans or native operation maps.

Runtime acceptance must cover reset after cancellation/failure, native-call
ordering, progressive PCM, actual fallback, multiple spans, both lanes and
reconnect. Only implemented components may be advertised. Changes to helpers
and public transport need full Emacsvox development staging; these fixtures and
audit tools alone do not change or deploy a runtime.
