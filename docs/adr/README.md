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
| [0011: Omnivox session configuration and reload](0011-session-configuration-and-reload.md) | Proposed | Omnivox coordinates configuration across independent workers; staged preference reload and later lifecycle Apply. |
| [0012: TGSpeechBox default distribution](0012-tgspeechbox-default-distribution.md) | Accepted | Default bundled helper, retained isolation and native platform qualification; partially supersedes 0001. |
| [0017: Independent host preference reload](0017-independent-host-preference-reload.md) | Accepted for implementation | Independent ordinary-worker preferences, coherent requests and separate native lifetime/asset safety. |
| [0018: Minimal owned emergency speech](0018-minimal-emergency-speech.md) | Accepted for implementation | Explicit bundled speech independent of management storage, with verified native retirement and retained asset claims. |
| [0019: Windows native creation ownership](0019-windows-native-creation-ownership.md) | Accepted for implementation | Guard-created workers/helpers, retained process completion and direct helper pipes; starts with explicit eSpeak/Flite qualification. |

Development main retains the earlier 0011 text and does not contain
the session implementation. Accepted session records 0011 and 0013–0016 at
`7879419` remain applicable; [0017](0017-independent-host-preference-reload.md)
links their exact retained texts and identifies which preference requirements it
supersedes. Their IDs remain reserved. Absence of those implementation modules
does not revoke their native ownership or asset protections.

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
