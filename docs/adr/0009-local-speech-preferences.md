# ADR 0009: Local speech and output preferences

- Status: Accepted
- Accepted: 2026-09-28.
- Consolidated: 2026-09-28 from the accepted local speech, saved defaults,
  capital-pitch and saved-output decisions, previously numbered 0009–0012;
  their policies are retained.
- Extends: [Engine registration](0008-extensible-engine-registration.md).
- Related: [Progressive playback](0003-progressive-audio-and-markers.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md),
  [PulseAudio](0005-native-pulseaudio-output.md),
  [actual-attempt tuning](0006-voice-selection-and-customization.md).
- Extended by: [Windows default output recovery](0010-windows-default-output-recovery.md).
- Proposed extension: [Session configuration and reload](0011-session-configuration-and-reload.md)
  would permit explicit preference reload with retained reset and recovery state.

## Context

Speech preferences and output choices need a persistent baseline while allowing
clients to make temporary changes and launchers to select each lane's channel.
Separate settings sources or file reads during reset and recovery could make
foreground and notification speech disagree. Reset must have a predictable
baseline, and retained owners must still be able to restart older executables.

## Decision

### Capture local preferences with engine startup

Use the existing strict configuration and immutable engine startup record from
ADR 0008. Public configuration version 2 adds optional speech and output settings;
version 1 and helper manifests retain their accepted meanings. Omitted settings
preserve existing built-in defaults. Validate configured values before native
construction, including values unused by the selected backend. A preference
cannot enable an excluded engine, invent a voice or relax a native limit.

Both workers share the captured file baseline. Recovery reuses that record and
each lane's retained launch choices. Reset and recovery never reread mutable
configuration; file edits require deliberate restart or activation. Historical
startup snapshots preserve their original defaults and serialized shapes so
retained owners can recover their pinned executables. Exact fields, bounds and
schema history belong to the [configuration reference](../reference/engine-configuration.md#configuration-version-2)
and [startup snapshot contract](../reference/engine-startup-snapshot.md).

### Keep speech defaults and host policy distinct

Saved voice, rate, pitch, gains and text-presentation defaults establish the
speech baseline. Startup command-line overrides and later client commands retain
their priority. Speech reset stops speech, clears pending work and transient
delays, and restores the captured file defaults; subsequent client commands may
override them again. Explicit voice styles keep their existing precedence.

Chunk size is a bounded host policy applied before engine selection to ordinary,
immediate, preview and structured speech. Preserve the 15-word default,
sentence/clause preference, UTF-8 source mapping and the same prepared chunk on
fallback. Reset retains this policy. Word count never replaces byte, PCM,
timeline-action, cancellation or ownership limits; requested silence remains
capped at 15 seconds and the accepted playback reserve is unchanged.

The isolated-capital cue uses a global absolute host pitch with optional
per-engine overrides or an off setting. Preserve the default cue of `1.5`; off
keeps ordinary pitch. Select the policy for each actual engine attempt, including
fallback, without mutating session pitch. Lowercase handling and word/sentence
capitalization actions retain their behavior. Relative multiplication would
change the existing cue when ordinary pitch changes and is not the chosen policy.

Exact speech diagnostics use the same captured defaults and existing explicit
overrides. Diagnostic WAV synthesis remains whole-text native synthesis; it is
not evidence of server chunking or text preparation. Speech gain is applied once
in the host pipeline. Detailed reset and diagnostic precedence remains in the
[saved-default contract](../reference/engine-configuration.md#saved-speech-defaults).

### Preserve per-lane output choices

Save the existing backend, channel and PulseAudio latency request in the same
configuration. Existing command-line output choices override launcher environment,
which overrides the file. Per-lane choices remain independent. Reject an
unsupported effective backend without silent fallback; a supported explicit
override can replace a platform-inapplicable file preference.

Speech reset restores the effective startup channel, including launch overrides,
across speech, tones and sounds. It neither reconstructs output nor changes its
latency request. The selected backend and latency remain process-lifetime choices,
retained through native reconnect. Following a Windows default endpoint within
that backend is the separate lifecycle decision in ADR 0010.

Startup and audio diagnostics share these choices. Processed WAV output applies
channel selection without opening a device; file playback reads configuration
without constructing speech engines or applying speech effects. Exact rules live
in the [output contract](../reference/engine-configuration.md#audio-output-settings).

## Consequences and alternatives

One captured configuration makes startup, reset and recovery predictable while
preserving temporary client settings and independent lane destinations. It adds
compatibility obligations for retained startup records. Resetting speech to
unrelated built-in values would defeat saved defaults; resetting channels to both
would discard launcher intent. Reading files during reset or reconnect could
silently change active speech or split the two workers.

Smaller synthesis windows add calls and can disrupt phrasing; larger windows may
delay the first result or reach existing action limits. Bounds are configuration
guards, not optimality claims. Pitch settings do not guarantee equal acoustic
cues, and requested buffering is not measured acoustic latency. Keep functional,
native and listening evidence distinct in the [evidence archive](../benchmarks/README.md).
Live reload, settings UI, named-device selection, remote configuration and changes
to safety limits remain outside this decision.
