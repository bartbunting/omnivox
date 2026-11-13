# Audio-device selection

- Status: Accepted for implementation on 2026-11-13 under
  [ADR 0020](../adr/0020-audio-output-device-selection.md). Dependency migration is
  checked in the [migration report](../benchmarks/2026-11-13-rodio-upgrade.md).
  Windows discovery, live Apply, host-scoped startup choices and the paired
  Emacsvox Apply/Save editor are implemented. The
  [Windows/client qualification](../benchmarks/2026-11-13-audio-device-selection.md)
  records full development staging, native and WSL checks, and remaining physical
  disconnect/listening acceptance. The feature is not a published release.
- Scope: Local independent main/notification destinations, explicit Apply/Save,
  and recovery that preserves the intended output.
- Entry: [Roadmap](../ROADMAP.md#feature-backlog).

## Delivery

1. Upgrade Rodio 0.19 to 0.23 and CPAL 0.15.3 to 0.18.2 while retaining current
   playback behavior. Keep WAV/Vorbis decoders, canonical PCM, bounds, source
   completion, cancellation and strict Windows default selection. Run audio and
   workspace tests, native-target compilation and Clippy before routing changes.
2. Add bounded read-only discovery and exact-ID startup
   selection for the Windows device backend. Discovery must not construct speech
   engines. Distinguish default selection, actual endpoint, missing selection and
   discovery failure. Keep friendly labels separate from opaque identity.
3. Add negotiated output-only Apply and observable per-lane state. Define exact
   request/result bounds, incarnation/revision checks, operation correlation,
   cancellation and old-peer behavior in the protocol reference before enabling
   the client. Retain the confirmed route through ordinary owned recovery without
   changing immutable engine startup records.
4. Add Emacsvox choices for main and notifications, scoped to the selected speech
   host/backend. Browsing does not change output. Apply and Save are distinct;
   unavailable and unsupported choices have useful spoken explanations. Preserve
   existing channel/spatial controls and other speech backends.
5. Stage the complete Windows development runtime and qualify both native Emacs
   and WSL-to-Windows. Keep the ordinary development/desktop installations intact
   until the candidate is ready. Record actual payload/source identities and
   distinguish automated completion from listening acceptance.
6. Extend selection and recovery to other device backends and native PulseAudio
   with their own evidence. A Linux WSLg server may expose only its virtual output;
   do not label that as enumeration of Windows physical devices. Mac qualification
   requires a native Mac environment.

## Compatibility and failure requirements

- Missing settings preserve existing defaults. CLI, environment and saved
  preferences keep their established precedence; an explicit follow-default
  choice must be able to override a missing saved device.
- Bind persisted IDs to the speech host and backend, not a UI enumeration index.
  Duplicate names, changed names, absent devices and incompatible host IDs must
  have deterministic outcomes. Bound counts and string sizes in public records.
- Old workers do not receive unsupported commands. Do not enable local discovery
  or Apply through the remote transport by accident. Diagnostic stdout remains
  parseable; operational logs go to stderr.
- Failed preparation preserves a healthy current route. Successful publication
  cancels its previous generation. Default changes cannot redirect an explicit
  choice. Reconnect cannot replay old requests or cues.
- Worker loss during Apply requires an explicit retained outcome or an unknown
  result; never infer success from timeout. Guard ownership and native retirement
  requirements remain in force during worker recovery.
- Output choice does not reset voice, speech rate, gains, punctuation, channel
  selection or engine eligibility. Device failure is not engine failure.

## Verification

Exercise exact PCM at cold start, idle and source boundaries; buffered and
progressive cancellation; bounded producers; format conversion; WAV/Vorbis
decoding; tones and sounds; and existing null/PulseAudio paths after the upgrade.

Use simulated backends for duplicate labels, absent IDs, default-change races,
failed opens, candidate loss before publication, revision races, full event queues,
disconnect/reconnect, shutdown and bounded retry. Verify truthful tickets and no
stale audio/cues on replacement. Cover missing/lost replies and worker recovery
before and after output publication.

Use client tests for accessible browsing, independent lanes, Apply versus Save,
saved missing devices, host scoping, old-server fallback and recovery overrides.
Compile affected Lisp and exercise a fresh client. Existing unrelated failures
must be recorded separately.

Native checks must identify the new runtime, not reuse an older Rodio payload.
Exercise speech, tones and sounds with Eloquence and DECtalk, plus a bundled
engine; reconnect during active speech and idle; and independent destinations.
Physical headset/speaker listening and acoustic latency remain distinct from
source completion and automated device notifications. Do not infer faster speech
from a dependency upgrade or unit-test duration.
