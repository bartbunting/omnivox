# Architectural decisions

These records explain durable choices and their tradeoffs. Read the
[architecture reference](../ARCHITECTURE.md) for current implemented behavior and
the [roadmap](../plans/NEXT_STEPS.md) for outstanding work. Accepted decisions
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
| [0008: Extensible engine registration](0008-extensible-engine-registration.md) | Proposed | Shared registration and explicit local configuration for independent helpers. |

The maintainer authorized a one-time consolidation and renumbering on 2026-09-27.
These are the replacement records; older numbers apply only to earlier Git
revisions. Pre-consolidation records are available in Git at `cdd6176` and are
not retained as parallel files or redirects. The replacement preserves accepted
policy; the extensibility decision remains proposed. Recorded test observations
were preserved in the [evidence archive](../benchmarks/README.md).

Numbers are stable after this baseline. For a task, read this index, the relevant
accepted records and their linked dependencies. Read all accepted records when
scope spans the architecture or affected dependencies are uncertain. Keep the
index and cross-links current when a decision is added or superseded.
