# ADR 0002: Measured speech-rate calibration

- Status: Accepted
- Consolidated: 2026-09-27 from the accepted common-rate and TGSpeechBox calibration decisions.
- Related: [Voice customization](0006-voice-selection-and-customization.md).

## Context

Native rate controls use incompatible units, midpoints and nonlinear responses.
Switching engines under the same normalized host rate should avoid unnecessary
speed changes while retaining each engine's actual limits.

## Decision

Map the host's existing `0.0`–`2.0` rate through an engine-specific monotonic,
piecewise-linear native-control table. The normal `0.5` target follows the
established Eloquence `v1` English curve for Emacsvox compatibility. Measure
canonical post-pipeline WAV duration; startup, model load, synthesis wall time
and playback latency do not define speech rate.

A representative qualified voice and corpus define each engine table; there
is no silently maintained independent calibration for every discovered voice.
RuTTS uses both built-in voices and calibrated eSpeak Russian through the
achievable same-language range, then the reference curve's relative high-rate
progression. TGSpeechBox Adam `en-us` uses the retained measured table, following
Eloquence through host `1.0` and saturating at its native `4x` ceiling from host
`1.2`. The earlier provisional TGSpeechBox mapping is no longer the policy.

Saturate honestly when a native engine reaches its maximum. Equal host values
are approximate across voices, languages and versions, not an acoustic guarantee
beyond available headroom. macOS retains its system-native mapping until a
native audit justifies calibration. New integrations need either measured curves
or an explicitly labelled provisional mapping.

Changes require retained before/after audits and tests for native bounds and
monotonicity. Keep measured corpora, executable/runtime identity and repetitions
with the results. The [calibration reference](../reference/rate-calibration.md) owns exact
tables, invocation and interpretation; the [audit archive](../rate-audits/README.md)
and [benchmark index](../benchmarks/README.md) preserve evidence.

## Consequences and alternatives

Calibration makes ordinary engine switches more consistent without changing
the public rate range. Native upgrades may need renewed measurements. A single
mathematical curve cannot represent the engines' different nonlinear responses;
unqualified midpoint mappings preserve avoidable speed differences. Resampling
completed audio merely to force equal duration harms intelligibility and timing
and is not the calibration strategy.
