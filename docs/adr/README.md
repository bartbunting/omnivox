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
| [0009: Local speech and output preferences](0009-local-speech-preferences.md) | Accepted | Frozen local settings, speech/reset precedence, capital cues and per-lane output choices. |
| [0010: Windows default output recovery](0010-windows-default-output-recovery.md) | Accepted | Follow default endpoint changes, retire interrupted speech and recover output without restarting engines. |

The approved 2026-09-27 consolidation replaced the earlier collection, available
in Git at `cdd6176`. The approved 2026-09-28 consolidation combined the local
configuration records, then numbered 0009–0012, into 0009 and renumbered Windows
output recovery from 0013 to 0010. The texts before that second consolidation
remain in Git at `3ff0dc0`; older numbers must be read in their revision's context.

Both consolidations preserve accepted policy. Exact configuration and snapshot
schemas remain in their references; test observations and raw artifacts remain
in the [evidence archive](../benchmarks/README.md). Superseded files and redirects
are not kept as a parallel collection.

Numbers remain stable. For a task, read this index, the relevant
accepted records and their linked dependencies. Read all accepted records when
scope spans the architecture or affected dependencies are uncertain. Keep the
index and cross-links current when a decision is added or superseded. Creating
any new ADR, including a proposed record, requires explicit approval in advance;
implementation approval alone does not authorize a new record.
The [documentation guide](../DOCUMENTATION-GUIDE.md#adr-lifecycle) defines the
threshold for an ADR, its lifecycle and the approved consolidation exceptions.
