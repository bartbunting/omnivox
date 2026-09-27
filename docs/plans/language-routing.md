# Language-aware voice selection

Status: Proposed; not authorized for implementation. This is a later increment
beyond the completed [engine configuration version 1](../reference/engine-configuration.md).
The [roadmap](../ROADMAP.md#extensible-engine-registration) tracks it.
[ADR 0008](../adr/0008-extensible-engine-registration.md) explicitly leaves new
language behavior outside its accepted first slice.

The aim is to keep speech in the requested language when a preferred voice is
unavailable, using explicit rules for any fallback. Current exact-match behavior
is documented in the [configuration reference](../reference/engine-configuration.md#language-selection).

## Proposed direction

This is a later versioned increment; none of the following adds a version-1
configuration field. Make language an explicit routing input with predictable
fallback, while preserving exact voice choices. Add this through negotiated
routing semantics; do not silently reinterpret existing selectors or change old
saved policies.

- Accept explicit language context for a speech request or span, with the
  logical voice's language and then a configured default as fallbacks. Capture
  this context with the admitted request, not mutable process-global state.
- Add ordered language rules selecting voices or engines, independent of how
  those helpers were installed. Prefer the requested language across eligible
  engines before permitting a change of language.
- Keep explicit physical choices authoritative. Exact previews remain exact.
  Language policy fills automatic choices and permitted fallback; it does not
  silently replace an explicitly chosen voice because metadata differs.
- Separate exact tag matching from language-range matching. Retain legacy exact
  matching; offer [RFC 4647 basic filtering](https://www.rfc-editor.org/rfc/rfc4647.html#section-3.3.1)
  for explicit range rules. A range `fr` can match `fr-CA` or `fr-FR`; `fr-CA`
  does not directly match `fr-FR`. An ordered rule can prefer `fr-CA`, then `fr`.
  Do not infer relationships between distinct languages or script variants.
- Within a language match tier, apply explicit voice/engine preferences, then
  deterministic existing defaults. For automatic routing, try the most specific
  configured language tier across its eligible engines before a broader tier.
- Make cross-language fallback an explicit policy choice: fail the route, or
  use a configured fallback and report language degradation. Unknown language
  metadata cannot satisfy a strict language requirement. Unlabelled requests
  retain ordinary configured default behavior.
- Expose requested language, matched voice language, resolution stage and any
  relaxation in bounded diagnostics. Preserve successful ordinary fallback
  when the policy permits it; do not label a different-language voice an exact
  match.

As a conceptual policy, a French-Canadian request might try a selected `fr-CA`
voice, another eligible `fr-CA` voice, then an explicitly permitted `fr` range.
Only a separate cross-language fallback rule would allow an English default.
The concrete storage and wire schema must make these stages explicit.

Initially, clients can use the existing language-bearing logical definitions
and property selectors. A later negotiated extension is needed for per-request
or per-span language context and changed matching/fallback semantics. Language
rules should extend the existing generation-safe policy, not introduce another
independently mutable routing table. Both speech workers and previews must use
the same admitted semantics.

Automatic detection and mixed-language segmentation remain optional later
features. They need confidence, override and short-text behavior of their own.
A future client detector can supply explicit context without changing the
helper registration contract. Engines needing an in-voice language argument
or multiple advertised languages require a negotiated descriptor/request
extension; selecting a voice is sufficient for the initial single-language
voice model.

## Decisions and acceptance before implementation

Define the storage and wire schemas, matching order, client language inputs and
cross-language fallback choices before implementation. Preserve old clients'
existing behavior and require negotiation for the new rules. Any change to an
accepted architectural decision needs a linked ADR under the
[documentation workflow](../DOCUMENTATION-GUIDE.md).

The later language increment requires its own negotiated schema and paired
client/server coverage for exact/range matching, region/script tags, unknown
metadata, explicit choices, cross-engine same-language fallback, cross-language
policy, previews and both workers. Its future configuration example must carry
a new schema number; version-1 readers reject it in full.
