# ADR 0006: Voice selection, tuning and private previews

- Status: Accepted
- Consolidated: 2026-09-27 from complete previews, layered tuning, variants and native-parameter decisions.
- Related: [Rate calibration](0002-speech-rate-calibration.md),
  [PCM commitment](0003-progressive-audio-and-markers.md),
  [managed eligibility](0007-managed-voice-lifecycle.md).

## Context

A logical voice is a portable ordered set of physical choices. Different voices
need different base settings, while contextual heading/emphasis rules must still
win. Runtime fallback can choose a voice that client-side prediction did not
expect. Previews must exercise actual routing privately without changing applied
configuration or falsely reporting a substituted exact voice.

## Decision

### Resolve and tune the actual synthesis attempt

Keep physical identity as `(engine_id, voice_id)` and retain stable choice IDs
even when selectors repeat. Layered logical definitions contain shared settings
and ordered sparse choice patches. At admission capture definition generation,
policy, administrative exclusions, host rate and span context. At each eligible
attempt compose shared settings, the selected choice's adjustment, then context.
Policy fallback has no choice patch. Preserve omission, zero, false, explicit
adapter default and inheritance; relative offsets are not added together.

Map the final common style once using the selected adapter's qualified defaults.
Rebuild native settings/effects from the admitted request for every attempt;
failed attempts cannot leak state. Mixed legacy/layered runs isolate effect state
at mode boundaries. Retain bounded attempts, health and pressure handling, and
the no-replay boundary after PCM commitment. Playback evidence distinguishes the
last attempted target, identities contributing accepted PCM and identities whose
frames actually reached source consumption.

### Keep exact and complete previews distinct

An exact selector preview cannot silently substitute another voice. A complete
voice preview uses a private logical definition and effective policy, so permitted
fallback can be a successful result. Capture the union of draft/applied exclusions
and refresh inventory/health without dropping them. Empty preference lists mean
Automatic. An expected host-rate check can reject a changed comparison base.

Use normal synthesis, playback tickets, bounds and cancellation. Previews never
replace applied voices, policy or global TTS state. Bound distinct committed
identities to 32 and the encoded response limit, report truncation, and aggregate
degradation only from accepted audio. Retain private choice identities without
echoing client palette IDs or entire chains.

### Let adapters describe typed native controls

Adapters own parameter meanings, units, ranges, reset behavior and dependencies
on common mappings. Clients own accessible editing and sparse persistence.
Native settings are typed data, never vendor commands or executable metadata.
A choice's native operation replaces a common-mapping output unless context
explicitly writes that output; context wins even if equal to the base value.
Restore a qualified pristine baseline between independent styles and failures.

Metadata lookup is asynchronous and cached by connection/runtime/profile. It
cannot load every model, restart speech or invoke installation. Catalogue-only
support does not promise writable native controls; each runtime is qualified
independently. Startup and engine-wide controls stay outside voice parameters.
Older servers use a common-only projection with explicit degradation; strict
customized previews cannot silently discard settings. Missing runtimes retain
saved inert preferences. Both speech workers acknowledge independently.

### Resolve bundled eSpeak variants on demand

Discover a bounded suffix catalogue in the running eSpeak engine while keeping
base inventory compact. Explicit `espeak:BASE+VARIANT` selection validates both
native IDs, their combined 39-byte native bound and exact realization. Automatic
property matching uses base inventory. Exact preview, routed speech and direct
synthesis share guards; exclusions cannot be bypassed by deriving a combination.
Missing variants can use a saved ordinary fallback, never a false exact preview.
Variant selection needs no model enable/Apply cycle or worker restart. Preserve
older saved IDs/readers and the established palette save/startup services.

### Negotiate every extension

Preserve old wire shapes and meanings. The control envelope stays at version 1;
new operation names and capability bundles distinguish complete previews,
layered tuning and native parameters. Layered tuning requires timeline 4/marker
3; native parameters use timeline 5/marker 4 and helper 6. On-demand eSpeak
variants have their own capability. Exact fields, limits and fixtures belong to
the [control reference](../protocols/control.md),
[layered contract](../protocols/voice-choice-tuning.org) and
[native contract](../protocols/engine-voice-parameters.md).

Advertise a capability only when its complete promised path works, including
registration, ordinary speech, private previews and truthful execution evidence.
Client and server fixtures must cover fallback, admission, mixed versions,
boundaries, both lanes and remote framing. Public protocol/helper changes require
Emacsvox's full Windows development staging; main-only payload reuse is
insufficient. Audible acceptance remains distinct from compiled/fixture checks.

## Consequences and alternatives

One admitted definition governs real fallback and previews without requiring
client inventory predictions to be correct. This adds explicit sparse-state,
versioning and evidence obligations. Shared-only tuning cannot express per-voice
defaults; flattening context before actual selection loses precedence. Silently
changing older preview or selector semantics breaks compatibility. Exposing
vendor command strings bypasses validation and native reset ownership.
