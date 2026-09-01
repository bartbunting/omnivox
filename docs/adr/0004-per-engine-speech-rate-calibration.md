# ADR 0004: Per-engine Speech-rate Calibration

- Status: Accepted
- Date: 2026-09-02

## Context

Omnivox exposes one normalized host speech rate, with `0.5` described as the
normal rate. Speech engines do not give that number a common acoustic meaning.
Some accept words per minute, some accept a multiplier, some accept an inverse
duration scale, and RHVoice accepts a signed relative control. Their nominal
midpoints produced substantially different speech rates. WinRT also had a
large acceleration immediately above host rate `0.5` because a single linear
mapping crossed two very different parts of its native range.

Using each engine's documented midpoint is predictable for an adapter author,
but it makes a logical voice change alter both voice and speed. Applying one
formula to every engine cannot correct different nonlinear response curves.

## Decision

### Calibrate measured engine curves separately

Each calibrated engine maps the normalized host rate through a monotonic,
piecewise-linear table expressed in that engine's native rate control. The
tables target the established Eloquence `v1` English curve because preserving
Eloquence operation is a compatibility requirement for Emacsvox users.

Calibration uses canonical post-pipeline WAV duration rather than synthesis
wall-clock time. Engine startup, model loading, helper startup, and audio
playback latency therefore do not affect a rate measurement. The repeatable
measurement procedure and current reference voices are recorded in
[speech-rate calibration](../RATE-CALIBRATION.md).

RuTTS uses a Russian corpus. It follows calibrated eSpeak Russian through the
range that eSpeak can realize, then applies the Eloquence curve's relative
high-rate progression to the remaining RuTTS headroom. Both built-in RuTTS
voices contribute to its shared calibration table.

### Preserve real engine limits

A native engine limit is a saturation point, not an error and not a reason to
distort lower rates. eSpeak, RHVoice, Piper, and some platform voices reach
their maximum before Eloquence. Their host-rate mapping remains monotonic and
saturates honestly. Documentation and capability descriptions must not claim
that equal rates are exact after one engine has saturated.

The host protocol continues accepting rates from `0.0` through `2.0`. This
decision changes their acoustic interpretation, not the protocol field or its
range.

### Keep claims evidence-based

Calibration is approximate and corpus-, language-, version-, and voice-
dependent. A representative stable voice defines an engine table; Omnivox
does not silently maintain a different table for every discovered voice.
Changes to a table require retained before-and-after audit reports and tests
for native bounds and monotonicity.

macOS AVSpeechSynthesizer retains its system-native mapping until the audit can
be run on macOS. Its existing direct mapping is not claimed to match the
calibrated reference curve merely because `0.5` is the platform default.

## Consequences

- Switching between calibrated engines at ordinary rates changes speed much
  less than using their unrelated native midpoints.
- WinRT no longer has its previous abrupt host-rate transition above `0.5`.
- Several engines synthesize faster at host rate `0.5` than they did before
  this decision.
- Engines with lower maximum rates stop accelerating before the host reaches
  `2.0`; that limitation is visible and testable.
- A future engine integration needs a measured rate curve or an explicitly
  documented provisional native mapping.
- Platform and engine upgrades can require recalibration when retained audit
  evidence shows a material change.

## Alternatives considered

### Keep native midpoint mappings

Rejected. It preserved adapter-local meanings while producing large audible
speed changes whenever routing selected a different engine.

### Apply one mathematical curve to every engine

Rejected. Measured engine responses were nonlinear in different places, and
their native ranges and saturation points differ.

### Resample completed audio to force equal duration

Rejected. Large post-synthesis time scaling harms intelligibility and marker
timing, while engines already expose native controls that preserve their own
prosody more effectively.
