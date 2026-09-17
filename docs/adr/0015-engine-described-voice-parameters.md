# ADR 0015: Engine-described native voice parameters

Status: Accepted for implementation on 2026-09-17

The maintainer approved continuing the Emacsvox design in commit `e0e83d71d`.
Keep common calibrated voice controls and add an adapter-owned typed catalogue
of native controls. The [wire contract](../engine-voice-parameters.md) and
[independent fixtures](../protocol-fixtures/engine-voice-parameters.json)
freeze the first implementation boundary. The
[Windows audit](../benchmarks/2026-09-17-native-voice-parameters.md) records its
runtime evidence and remaining qualification limits.

The actual engine adapter owns parameter meanings, units, limits, reset
behavior, native application ordering and common mapping dependencies.
Emacsvox owns accessible editing and sparse per-choice persistence. Native
extensions are typed data, never vendor command strings or executable metadata.
Helpers retain their isolated native ownership under ADR 0001.

Extend ADR 0011's shared, selected choice, contextual precedence at native
parameter granularity. Map the final common style once. A native choice
operation replaces an output unless an explicit contextual mapping writes that
output. Context wins even when its value equals the base. Preserve omission,
zero, false and explicit engine-default operations. Restore a qualified pristine
voice baseline before each independent style; previews and failed attempts must
not leak native state. Recompose for the actual fallback choice.

Use control envelope 1 with new operation names, timeline 5, marker events 4,
and helper protocol 6. Preserve all earlier request shapes and meanings.
Catalogue support alone is read-only; advertise `engine_voice_parameters_v1`
only when registration, ordinary speech, private previews, actual native
execution and evidence all work. Qualify each helper independently. Preserve
ADR 0006's PCM commitment, backpressure and cancellation rules and ADR 0008's
per-connection ownership. Keep ADR 0004's existing common rate calibrations.

Cache metadata by connection/runtime/profile and request it asynchronously.
Catalogue access cannot load all voice models, restart speech, or invoke
installation management. Engine-wide and startup controls remain separately
scoped; the first voice-parameter contract cannot mutate those settings.
Voice assets, enabled status and residency retain ADRs 0012–0014 ownership.

Older targets retain ordinary speech through the best supported common
projection with explicit native degradation. Strict customized previews cannot
drop unsupported native settings. Missing runtimes preserve saved inert data.
Main and notification workers acknowledge application independently.

Typed metadata, validation and pure native planning are implemented in
`omnivox-tts::native_parameters`. Native helper execution and public operations
remain pending; no new wire capability is advertised.
Native helper/protocol implementation will require full development staging
through Emacsvox under ADR 0010. No dependency, runtime distribution, release
pin or publication change is authorized by this decision.
