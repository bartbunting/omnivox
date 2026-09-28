# Architectural decisions

These records explain durable choices and their tradeoffs. Read the
[architecture reference](../ARCHITECTURE.md) for current implemented behavior and
the [roadmap](../ROADMAP.md) for outstanding work. Accepted decisions
constrain implementation; acceptance alone is not evidence that a feature ships.
Proposed records do not override accepted decisions.

| ADR | Status | Scope |
| --- | --- | --- |
| [0001: Engine isolation and distribution](0001-engine-isolation-and-distribution.md) | Accepted | Native process boundaries, runtime supply and component release constraints. |
| [0002: Speech-rate calibration](0002-speech-rate-calibration.md) | Accepted | Measured common-rate mappings and honest saturation. |
| [0003: Progressive audio and markers](0003-progressive-audio-and-markers.md) | Accepted | Bounded PCM, source clocks, commitment, anchors and playback reserve. |
| [0004: Workstation service and worker ownership](0004-workstation-service-and-worker-ownership.md) | Accepted | Remote boundary, authentication, independent lanes and retirement. |
| [0005: Native PulseAudio output](0005-native-pulseaudio-output.md) | Accepted | Optional backend, priming, idle/stop and output recovery. |
| [0006: Voice selection and customization](0006-voice-selection-and-customization.md) | Accepted | Actual-choice tuning, private previews, typed native controls and variants. |
| [0007: Managed voice lifecycle](0007-managed-voice-lifecycle.md) | Accepted | Installed/desired/active state, providers, validation, activation and fallback. |
| [0008: Extensible engine registration](0008-extensible-engine-registration.md) | Accepted | Shared registration and explicit local configuration for independent helpers; configuration v1 and its first implementation slice. |
| [0009: Local speech preferences](0009-local-speech-preferences.md) | Accepted | Bounded host chunk size, unchanged defaults and frozen startup compatibility. |
| [0010: Saved speech defaults](0010-saved-speech-defaults.md) | Accepted | Saved startup/reset values, client precedence and frozen compatibility. |
| [0011: Capital pitch preferences](0011-capital-pitch-preferences.md) | Accepted | Global and per-engine isolated-capital cues, actual-attempt selection and frozen compatibility. |
| [0012: Saved audio output](0012-saved-audio-output.md) | Accepted | Saved backend/channel/latency, launcher precedence, reset destination and frozen recovery. |
| [0013: Windows default output recovery](0013-windows-default-output-recovery.md) | Accepted | Follow default endpoint changes, retire interrupted speech and recover output without restarting engines. |

The maintainer authorized a one-time consolidation and renumbering on 2026-09-27.
These are the replacement records; older numbers apply only to earlier Git
revisions. Pre-consolidation records are available in Git at `cdd6176` and are
not retained as parallel files or redirects. The replacement preserves accepted
policy; the extensibility decision was subsequently accepted for its v1 scope. Recorded test observations
were preserved in the [evidence archive](../benchmarks/README.md).

Numbers are stable after this baseline. For a task, read this index, the relevant
accepted records and their linked dependencies. Read all accepted records when
scope spans the architecture or affected dependencies are uncertain. Keep the
index and cross-links current when a decision is added or superseded.
The [documentation guide](../DOCUMENTATION-GUIDE.md#adr-lifecycle) defines the
threshold for an ADR, its lifecycle and the one-time consolidation exception.
