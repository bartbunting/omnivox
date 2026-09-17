# ADR 0014: On-demand bundled eSpeak variants

- Status: Accepted by the maintainer on 2026-09-17

Bundled eSpeak variants do not use the downloaded-model enable/Apply lifecycle.
Discover a bounded suffix catalogue once inside the running eSpeak engine and
expose it as optional `espeak_variants` metadata in its engine descriptor. The
`espeak_variants_v1` capability promises exact on-demand combination resolution
for ordinary speech, exact previews and complete palette previews. It adds no
request operation, helper protocol version, process boundary or dependency.

Keep base voice inventories compact. Resolve a valid `espeak:BASE+VARIANT` only
when explicitly requested; automatic/property routing continues choosing from
ordinary advertised voices. Validate both native IDs, the 39-byte native bound,
and exact native realization. Native selection and synthesis retain the existing
lock, cancellation and PCM commitment boundaries. Explicit inventory exclusions
override derived combinations; engine disablement and voice-library exclusions
remain authoritative. Missing combinations fail normally and may use a saved
fallback, while exact previews must never claim base-only substitution.

Retain old physical IDs and startup configuration readers for compatibility.
Old enabled choices may remain explicit inventory rows; the old list no longer
gates other valid combinations. Emacsvox stops publishing the obsolete enable
setting and uses the running worker's catalogue without a separate discovery
process. Preview writes nothing. Palette editing retains existing choices and
tuning and uses the established save/apply and startup-selection services.

All prior ADRs still govern downloaded-model lifecycles, bounded protocols,
engine isolation, both speech lanes and deployment. This inventory/capability
extension receives full Windows development staging and matching client/server
checks, with older servers explicitly lacking on-demand variant support.
