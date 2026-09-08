# ADR 0010: Negotiated complete-voice previews

Status: Accepted
Date: 2026-09-08

## Context

The Emacsvox voice editor owns ordered physical choices and shared tuning in
one draft. The existing `preview` operation auditions one selector with an empty
private fallback policy. Extending that request silently would make older
clients and servers disagree about which voice was auditioned.

## Decision

Keep the version-1 envelope and the existing `preview` / `preview_completed`
operations. Advertise `voice_chain_preview_v1` for the separate `preview_voice`
request and `preview_voice_completed` terminal response.

Admission captures a private logical definition, the client's complete effective
fallback policy, the current host rate, and the union of draft and applied
administrative disablements. It never replaces live logical voices, routing
policy, or TTS state. The worker refreshes inventory and runtime health while
preserving captured disablements. Empty preferences mean Automatic.

Use normal routed synthesis, playback tracking, queue bounds, cancellation and
fallback. ADR 0006's PCM commitment boundary remains authoritative: an output
failure or a synthesis failure after committed PCM cannot replay via fallback.

Terminal metadata distinguishes the last physical target from ordered distinct
identities that actually supplied audio accepted by playback. Bound the latter
to 32 identities and the existing encoded-control limit, declaring truncation.
Aggregate degradation from accepted audio only. Preserve correlation for bounded
validation errors and queue retirement. An optional expected host rate rejects a
changed comparison base before synthesis.

## Consequences

Clients must negotiate on each actual connection before sending complete
previews. Successful fallback is a valid preview result and must not be rejected
as an exact-selector mismatch. Clients retain the submitted draft and policy;
responses do not echo potentially large chains or expose palette/local IDs.

The control protocol reference and paired JSON/wire fixtures define the public
shape. Server tests execute the fixtures with simulated engines and exercise
fallback, partial failure, Automatic, disablement, rate, bounds and isolation.
Real engine listening and frontend comparison lifecycle acceptance remain
separate from these server checks. No helper protocol or dependency changes are
required. A Windows development deployment needs the full protocol-aware
staging path, not main-only payload reuse.
