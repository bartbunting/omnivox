# Richer generated tones

Status: server support published in 1.18.0; client delivery and qualification
follow-up remain active. Reviewed 2026-10-23. Both `rich_tones_v1` and
`rich_tones_v2` are implemented, with native release gates and Windows x64
development-runtime listening acceptance recorded in the
[release evidence](../benchmarks/README.md#omnivox-1180-release-evidence).
Emacsvox's shared editor covers spelling, capital-letter/all-caps cues and
pitch-based indentation. Shared-editor listening acceptance was recorded on
2026-10-17; destination-specific listening and broader platform/acoustic
acceptance remain open. Classic defaults are unchanged.
Scope: richer generated cues in Omnivox, with accessible editing in Emacsvox.
Related: [roadmap](../ROADMAP.md), [architecture](../ARCHITECTURE.md),
[timeline protocol](../protocols/presentation-timeline.md),
[control protocol](../protocols/control.md),
[bounded audio and markers](../adr/0003-progressive-audio-and-markers.md),
and [local speech and output preferences](../adr/0009-local-speech-preferences.md).

## Aim and current boundary

Make spelling, capitalization, indentation and other brief cues easier to
distinguish and comfortable to hear repeatedly. Offer understandable presets
alongside optional detailed controls. Preserve existing cues by default.

The active Classic [generator](../../omnivox-audio/src/tone.rs) produces sine waves
with linear fades of up to 10 milliseconds at each end, shortened for brief
tones. It produces stereo PCM with a small right-channel delay. Negotiated
standalone and timeline tones share individual gain, waveform and envelope
definitions. Per-cue gain is multiplied once by the worker's master tone volume.
Emacsvox edits individual gain and shape while retaining separate master controls.

Keep synthesis in the existing audio pipeline. Do not introduce a speech engine,
helper process, external synthesizer or new dependency merely to generate cues.
Preserve stream ownership, routing, action order, resource budgets and terminal
playback reporting. Existing startup sounds and legacy callers keep their sound.

<a id="proposed-tone-model"></a>

## Implemented tone model

The [protocol reference](../protocols/presentation-timeline.md#richer-tones-version-6)
owns exact fields, bounds and rendering rules; this summary describes the
delivered scope.

- **Waveform:** sine, triangle, square, variable-width Pulse and Sawtooth, with
  bounded harmonic generation for the non-sine waveforms.
- **Frequency:** fundamental pitch in hertz, with optional linear pitch sweeps
  in `rich_tones_v2`. Older commands retain their validation and behavior.
- **Duration:** total audible allocation in milliseconds, including the envelope.
- **Gain:** normalized per-cue amplitude, multiplied once by existing master
  tone gain. Preserve the current routing, pan and effect-bus contracts.
- **Envelope:** attack time, decay time, sustain level and release time. Attack
  rises from silence to peak; decay falls to sustain level; sustain occupies
  remaining time; release falls to silence. Sustain is a level, not a duration.

If attack, decay and release exceed the total duration, the renderer scales
them proportionally and gives sustain zero hold time. Deterministic frame
allocation, zero-stage handling and endpoint smoothing preserve fixed duration.
The version-2 bundle adds exponential envelope curves alongside linear curves.

Provide a **Classic** rendering path that preserves current samples, fades and
stereo behavior when new fields are absent. Generalized ADSR rendering must not
quietly redefine old tones. Emacsvox provides named presets and current-tone
comparison. Further preset/gain balancing needs listening evidence. Equal peak
amplitude does not establish equal perceived loudness.

## Compatibility and persistence

Negotiate richer-tone support per worker before sending new parameters.
`rich_tones_v1` pairs standalone version 2 and timeline version 6;
`rich_tones_v2` pairs standalone version 3 and timeline version 7. The
[protocol reference](../protocols/presentation-timeline.md#extended-tones-version-7)
defines the advertised bundles. Both paths render the same selected definition
and effective gain.

Old clients keep existing commands and defaults. On an older server, continue
to offer Classic controls. For a saved richer cue, Emacsvox omits unsupported
tones while preserving speech and reporting the reason; exact previews report
missing support. It retains saved intent without silently substituting Classic.
Check capabilities per connected worker, including remote workers.

Validate finite values, ranges, combinations, encoded sizes and generation
cost before admission. Reject malformed or unsupported explicit requests without
silently ignoring fields or disturbing existing work. Keep generated PCM and
effect tails within existing presentation/window budgets. Harmonic generation
must have bounded CPU cost as well as bounded memory.

Emacsvox stores ordinary cue defaults and presets in Customize; conditional
personal rules remain in its presentation store. Extended personal-rule shapes
use schema 13. Opening, browsing and previewing save nothing. Capitalization
Save migrates only unchanged menu-generated rules, preserving edited rules.
The paired [UI plan](https://github.com/bartbunting/emacsvox/blob/main/docs/ui-settings-plan.org) owns client
editing, scope, persistence and compatibility details.
This review used Emacsvox commit `e36fa1e35`.

## Delivery slices

1. [x] **Individual cue volume in the initial destinations.** Spelling,
   capital-letter/all-caps and pitch-based indentation expose individual gain.
   Standalone previews, timeline feedback and retained replay carry frozen gain;
   master volume remains separate. Other destinations remain follow-up work.
2. [x] **Contract and fixtures.** Audit all callers; settle the duration rule,
   short/zero-stage behavior, waveform limits, capability/version negotiation,
   old-peer fallback and storage compatibility. Define standalone/timeline
   parity and negative fixtures before exposing controls.
3. [x] **Omnivox generation and transport.** Implement Classic preservation,
   waveforms and envelopes in the shared renderer. Wire both delivery paths
   through it. Verify bounded synthesis and cancellation before client rollout.
   Platform and listening qualification remain in slice 5.
4. [x] **Emacsvox editor and presets for the initial destinations.** The common
   draft editor, presets, current-tone comparison, Preview/Stop, Apply, explicit
   Save and discard are implemented for spelling, capitalization and pitch-based
   indentation. Shared-editor listening was accepted on 2026-10-17;
   destination-specific acceptance remains in slice 5.
5. [ ] **Qualification and documentation.** Test standalone, inserted and
   overlaid tones, rapid preview replacement, both speech lanes and remote
   delivery. Start listening with spelling, then capital-letter/all-caps and
   indentation cues. Record platform/runtime coverage separately from unit
   results. Update supported protocols, guides and status only as implemented.
6. [x] **Tone extension implementation, accepted 2026-10-16.** The renderer and
   versioned decoders implement bounded linear pitch
   sweeps, linear/exponential envelope curves, Pulse with 5–95 percent width
   and Sawtooth. Preserve older formats through standalone version 3 and
   timeline version 7, negotiated as `rich_tones_v2`. The exact contract lives
   in the [protocol reference](../protocols/presentation-timeline.md#extended-tones-version-7).
   Named presets and comparison remain client features. Short sequences, chords,
   noise, modulation and arbitrary curves remain deferred.

Next work is destination-specific listening for capitalization and pitch-based
indentation, broader device/platform and remote qualification, and physical
onset/stop measurements. General personal-rule and line-cue editors and
duration-coded indentation remain separate client follow-ups. Passing native
release gates does not close these listening and interaction checks.

Commit implementation in reviewable slices. Keep Omnivox and Emacsvox changes
separately identifiable and qualify the paired versions before deployment.
Protocol changes use the required full development staging route, not a
main-only shortcut. This plan does not authorize deployment or publication.

## Verification and listening acceptance

Automated checks must cover:

- Unchanged Classic samples and legacy behavior, omitted fields and old peers.
- Exact frame counts, short envelopes, zero stages, bounds and finite output.
- Waveform frequency content and alias suppression, amplitude scaling and
  existing stereo/channel routing; master gain must not be applied twice.
- Identical definitions through previews and actual feedback, including saved
  defaults and retained replay evidence.
- Insert/overlay timing, window boundaries, resource limits and bounded CPU.
- Rapid replacement, stopped previews, both workers and stale work after
  cancellation. Normal release time must never delay a stop: preserve the
  current cancellation policy, including the short active-tone cancellation
  fade, and truthful terminal status.
- Graphical Emacs menu selection, Help, interruption, draft retention,
  explicit save/discard, older-backend feedback and restart persistence.

Listening acceptance must separately check:

Windows x64 runtime `c7a933a8824837b6` received overall listening acceptance on
2026-10-16, and Emacsvox's shared editor on 2026-10-17. The detailed criteria
below remain the checklist for broader coverage: those overall confirmations
do not supply a per-waveform/device matrix or accept every new destination.

- [ ] Classic sounds unchanged in familiar spelling, caps and indentation use.
- [ ] Soft, Plucked and Rounded are distinguishable and their descriptions fit.
- [ ] Brief cues do not click, become unexpectedly long or disappear audibly.
- [ ] Changing waveform does not produce an unpleasant loudness jump.
- [ ] Repeated spelling and navigation cues remain comfortable and do not
  obscure speech; per-cue gain and master gain have understandable effects.
- [ ] Preview Stop and rapid navigation interrupt promptly, including when a
  long normal release is selected.
- [ ] Preview, live feedback, saved/restarted cues and replay agree where the
  retained record supports that claim.

Use matched builds and record actual backend/device/platform paths. Null-output
and captured PCM checks do not prove acoustic quality or physical latency.

<a id="implementation-contract-and-effort-handoff"></a>

## Implemented contract and retained verification

The following records the implemented first bundle and its compatibility
constraints. Both bundles are now advertised; the
[protocol reference](../protocols/presentation-timeline.md#extended-tones-version-7)
also specifies the shipped extensions. Remaining work is client follow-up and
qualification, not the original decoder/renderer patches.

### Wire and capability boundary

Standalone tone version 1 and timeline versions 1 through 5 remain unchanged.
Standalone tone version 2 and timeline version 6 are gated by the complete
`rich_tones_v1` capability. Timeline 6 reuses version 5 speech, registration,
marker and framing semantics and adds an optional `shape` to tone actions.
Do not widen the shared older action reader to accept the new shape implicitly.
An omitted shape renders Classic. Unknown nested fields and invalid values
are errors. Advertise the bundle only after both paths render and stop correctly.

Version 2 standalone forms are:

```text
emacsvox_tone 2 MODE FREQUENCY DURATION GAIN WAVEFORM classic
emacsvox_tone 2 MODE FREQUENCY DURATION GAIN WAVEFORM adsr ATTACK DECAY SUSTAIN RELEASE
```

MODE remains `insert` or `overlay`; WAVEFORM is `sine`, `triangle` or `square`.
Times are integer milliseconds, gain and sustain are finite numbers in 0..1.
Duration is 1..60000 ms; each envelope time is 0..60000 ms. Richer rendering uses
frequencies greater than zero through 20000 Hz. Existing Classic commands retain
their 24000 Hz limit. Gain alone with Classic must also preserve that old range.

Timeline shape contains `waveform` and optional `envelope`; the envelope contains
`attack_ms`, `decay_ms`, `sustain_level` and `release_ms`. Omitted envelope uses
Classic fades. Reject explicit nulls, malformed combinations and trailing
standalone arguments. Keep existing action gain, mode, pan and effect-bus fields.
Use one typed validator for both new delivery paths.

Once a worker negotiates version 6 or 7, use the selected version consistently
for its structured timelines, including those with Classic or no tones. Do not alternate versions
according to cue settings: replacement domains include the protocol version.
Carry the existing registration and delivery guards through the new version.
Verify worker-local negotiation, remote framing and reconnect independently.

For an older worker, keep ordinary Classic cues. A non-default tone gain or
shape must not silently become a louder Classic tone. Retain the requested
settings, omit that unsupported cue from ordinary feedback with explicit
degradation evidence, and preserve speech and other actions. An explicit preview
instead reports the missing capability without playing an inaccurate sample.
Explain this limitation in menu Help; never simulate cue gain by temporarily
changing the worker's global tone volume.

### Rendering and cancellation

Keep the current sine/Classic implementation as the reference path, including
stereo delay and exact frame count. Bound triangle and square synthesis to at
most 32 odd harmonics below Nyquist. Use fixed amplitude bounds rather than
automatic loudness matching; spectral tests and listening qualify the result.
If that bounded additive implementation is too costly, select another bounded
algorithm before exposing it, preserving the same wire contract.

For ADSR, proportionally fit attack, decay and release into the total duration
when necessary; sustain occupies the remaining time. Allocate transitions over
the available sample intervals so the final frame reaches zero. A zero-length
stage jumps to the following stage. Apply an additional smooth endpoint cap of
up to 1 ms, shortened to half the tone for very brief cues; take the minimum of
this cap and the ADSR level rather than multiplying two envelopes. This endpoint
cap never extends duration. Classic samples bypass the new envelope path.

Check both hard-generation and keyed cancellation before allocation, at most
every 256 generated frames, and before publishing the finished buffer. Discard
partial PCM. Cancellation is a distinct outcome, never a synthesis failure or
a reason to queue dry speech. Preserve current active-source cancellation fades;
normal envelope release never delays Stop.

Pass the cancellation probe through `prepare_speech_timeline_actions` and
`TimelineAudioResource::materialize`, covering buffered, prepared and progressive
paths as well as queued standalone tones. The timeline-render error handler
keeps cancellation separate from failures that can queue dry speech.
Tests cover cancellation during materialization, not just an already-cancelled
request or active playback.

<a id="client-and-next-patches"></a>

### Client delivery and retained checks

The following retains implementation verification and provenance. Its muted
checks preceded the Windows listening acceptance recorded above; test counts
describe those runs and are not a claim that checks were rerun for this review.

The decoder slice adds [shared typed values](../../omnivox-core/src/tone.rs),
the [standalone decoder](../../omnivox-core/src/tone_command.rs), the
[timeline decoder](../../omnivox-tts/src/timeline_v6.rs), and a
[shared wire fixture](../protocol-fixtures/richer-tones.json). Regression tests
cover matching sound definitions, Classic omission, old-version rejection,
unknown and duplicate fields, explicit nulls, numeric bounds, short envelopes,
payload limits and replacement domains. These checks establish decoding only;
they do not establish playback or listening acceptance.

The [shared renderer](../../omnivox-audio/src/tone/rich.rs) now implements sine,
triangle and square waveforms, fixed-duration ADSR and endpoint smoothing.
Classic calls retain their sample calculations. Harmonic generation uses at
most 32 odd partials below Nyquist; square coefficients use one fixed absolute
sum bound across frequencies. This keeps amplitude bounded without matching
perceived loudness. Relative waveform loudness still needs listening.

Cancellation probes now reach existing standalone, capitalization and timeline
tone generation, including progressive preparation and actual-attempt effects.
Generation checks before allocation, every 256 frames and before returning PCM.
Cancelled preparation drops partial audio without marking the request failed
or queuing dry speech. Existing playback cancellation fades are unchanged.
Regression coverage checks spectra, Classic sample equality, gain, envelope
stages and compression, silent endpoints, cancellation during materialization,
and cancellation handling for buffered and progressive preparation.

Both versioned formats now reach playback. Tests compare standalone PCM with
inserted and overlaid timeline PCM across waveforms, envelopes and channel
routing, using separate cue and master gains. Reader-to-worker tests cover
buffered and progressive fallback with retained marker evidence. Multipart
framing preserves shapes; malformed input cannot cancel accepted work;
version-six Classic updates replace richer predecessors without cancelling
version-five work or another key. The
[protocol reference](../protocols/presentation-timeline.md#richer-tones-version-6)
now defines the implemented wire contract and negotiated bundle.

Linux workspace verification passes: 1,187 tests passed, with the two existing
qualification/stress tests ignored. Formatting, Clippy and paired documentation
checks pass. Full Windows development staging passes for build
`a4b085773844514f`, from Omnivox commit `a04e91e`. A fresh native Windows process
through the normal launcher, using null audio output, confirms capability
negotiation and standalone, direct-timeline and multipart-timeline admission
and completion. This run verifies transport and execution, not audible output;
the later listening confirmation is recorded separately above.

Emacsvox now negotiates the richer-tone bundle independently for each worker,
including reconnects. Standalone version 2 and timeline version 6 carry frozen
per-tone gain, and original replay retains that value. Rules validate literal
and fact-derived gain. Older workers omit unsupported tones while preserving
speech and recording the reason; exact previews report missing support. These
first two client slices are committed as `47df2aa1a` and `82d13b353`.

Spelling now has individual volume at `C-e H s f l`, with 100 percent preserving
the existing level. The shared main-tone volume still scales it. Reading,
editing feedback and previews use the same gain; explicit Save persists it
through Customize. Personal rules containing tone volume use user-data schema
11; existing schemas remain readable. Saving native voices preserves schema
11 when tone rules are present.

Client regression checks cover worker isolation, old-worker degradation,
frozen gain, saving and replay. Fresh graphical Emacs checks cover adjustments,
Enter activation and search. The compiled client completes muted native Windows
checks on both workers and after reconnect, including standalone and timeline
gain and retained replay. The staged runtime is reused without a server rebuild.
The Emacsvox UI plan records exact test counts and an outstanding test-order
issue in an unrelated indentation fixture. This checkpoint did not establish
audible acceptance; later confirmations are recorded above.

Emacsvox now preserves waveform and envelope fields in frozen actions,
validation, saving and replay. Its spelling editor includes named presets,
draft/current comparison and the extended controls. Customize owns saved cue
defaults and presets; extended personal-rule shapes use schema 13.
The common editor now also serves capital-letter/all-caps and pitch-based
indentation cues. Their destination-specific listening remains open. Old workers
must keep reporting unsupported shapes without changing saved intent or silently
playing Classic.

Preset values remain provisional listening choices, with no automatic loudness
compensation or default changes. Initial trials can use Soft (10/0/1/15 ms),
Plucked (1/25/0.25/10 ms), and Rounded (10/10/0.7/15 ms), expressed as
attack/decay/sustain/release; sustain is unitless. Classic remains the default.

No new ADR is created by this plan. If the contract work requires changing an
accepted architectural boundary, identify that decision and obtain separate
approval for its ADR before proceeding.

Extension verification covers strict older formats, matching standalone and
timeline definitions, fixed duration, width-dependent sound, 50-percent
Pulse/Square parity, sweep frequency, exponential levels, cancellation during
generation, mixed-version replacement and multipart admission. Workspace tests,
Clippy, formatting and paired documentation checks pass. Full development
staging produced Windows x64 runtime `c7a933a8824837b6` from commit `6e57e7d`.
A fresh compiled Emacsvox client passes muted standalone version 3 and timeline
version 7 checks for Pulse widths 5, 50 and 95 percent, rising and falling
sweeps, exponential ADSR and Sawtooth. Checks cover insert and overlay on both
workers, reconnection and historical speech replay. The normal Emacsvox launcher
selected this runtime for that verification. These muted checks establish
execution and protocol behaviour. The later Windows listening confirmation and
native release gates are recorded in the
[release evidence](../benchmarks/README.md#omnivox-1180-release-evidence);
broader listening and acoustic timing remain open.
