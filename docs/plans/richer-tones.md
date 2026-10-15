# Richer generated tones

Status: accepted for implementation on 2026-10-16. Shared tone types, versioned
decoders, waveform/envelope rendering and cancellation during tone generation
are implemented. Standalone version 2 and timeline version 6 now reach playback
through the same renderer, advertised together as `rich_tones_v1`.
Client controls, old-worker degradation,
platform qualification and listening acceptance remain outstanding. Defaults
are unchanged.
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
tones. It produces stereo PCM with a small right-channel delay. Timeline tones
already accept individual gain, multiplied by the worker's master tone volume.
Emacsvox currently supplies unity gain for each timeline tone and exposes the
master control. Standalone presentation-tone commands do not offer all the
fields available to timeline actions.

Keep synthesis in the existing audio pipeline. Do not introduce a speech engine,
helper process, external synthesizer or new dependency merely to generate cues.
Preserve stream ownership, routing, action order, resource budgets and terminal
playback reporting. Existing startup sounds and legacy callers keep their sound.

## Proposed tone model

- **Waveform:** initially sine, triangle and pulse/square. Select an algorithm
  that controls aliasing for the harmonic waveforms; do not ship a naive hard
  discontinuity as the default implementation. Pulse width is deferred.
- **Frequency:** the fundamental pitch in hertz. The richer path needs limits
  consistent with the renderer's sample rate and waveform generation. Existing
  commands retain their accepted validation and behavior.
- **Duration:** total audible allocation in milliseconds, including the envelope.
- **Gain:** normalized per-cue amplitude, multiplied once by existing master
  tone gain. Preserve the current routing, pan and effect-bus contracts.
- **Envelope:** attack time, decay time, sustain level and release time. Attack
  rises from silence to peak; decay falls to sustain level; sustain occupies
  remaining time; release falls to silence. Sustain is a level, not a duration.

Duration rule for implementation: if attack, decay and release exceed the total
duration, scale those three times proportionally and give sustain zero hold
time. Allocate frames deterministically so rounding cannot extend the tone.
Very short cues must remain valid. Specify zero-length stages and endpoint
handling before implementation; define how click suppression affects a requested
instantaneous edge. Do not silently add time beyond the requested duration.

Provide a **Classic** rendering path that preserves current samples, fades and
stereo behavior when new fields are absent. Generalized ADSR rendering must not
quietly redefine old tones. A proposed preset list is Classic, Soft, Plucked and
Rounded. Presets describe envelopes; waveform remains a separate choice.
Exact preset values and gain balancing require listening. Equal peak amplitude
does not establish equal perceived loudness.

## Compatibility and persistence

Negotiate richer-tone support before sending new parameters. Freeze exact
capability names, wire fields, versions, bounds and error responses in the
contract slice; they are not defined or advertised by this proposal.
Cover both structured timeline actions and standalone previews/commands.
Both paths must render the same selected definition and effective gain.

Old clients keep existing commands and defaults. On an older server, continue
to offer Classic controls. For a saved richer cue, expose the unavailable
capability and an explicit Classic fallback policy rather than pretending the
chosen sound was rendered. Settle that policy before enabling richer saved
definitions. Check capabilities per connected worker, including remote workers.

Validate finite values, ranges, combinations, encoded sizes and generation
cost before admission. Reject malformed or unsupported explicit requests without
silently ignoring fields or disturbing existing work. Keep generated PCM and
effect tails within existing presentation/window budgets. Harmonic generation
must have bounded CPU cost as well as bounded memory.

Emacsvox owns named cue definitions and personal overrides through its existing
presentation-rule persistence. Master volume retains its existing owner.
Version stored richer definitions deliberately, preserve older definitions on
load and avoid rewriting personal files merely by browsing. Clarify what older
Emacsvox versions can read before shipping the new saved format.

## Delivery slices

1. [ ] **Individual cue volume in Emacsvox.** Expose existing timeline gain for
   spelling first, then other tone editors. Trace standalone previews, replay
   and actual feedback so they use the same gain. If a standalone path cannot
   express it, use an existing equivalent negotiated path or carry that gap
   into slice 2 explicitly. Keep the shared master volume clearly labelled.
2. [x] **Contract and fixtures.** Audit all callers; settle the duration rule,
   short/zero-stage behavior, waveform limits, capability/version negotiation,
   old-peer fallback and storage compatibility. Define standalone/timeline
   parity and negative fixtures before exposing controls.
3. [x] **Omnivox generation and transport.** Implement Classic preservation,
   waveforms and envelopes in the shared renderer. Wire both delivery paths
   through it. Verify bounded synthesis and cancellation before client rollout.
   Platform and listening qualification remain in slice 5.
4. [ ] **Emacsvox editor and presets.** Use browsable waveform and envelope
   choices: arrows inspect, Enter applies to the draft. Explain every choice
   and numeric control. Use Shift-arrows for adjustments and Enter for exact
   values. Provide explicit Preview, Stop, reset/discard and Save with scope
   stated. Reuse existing drafts and rules; do not change defaults on entry.
   Ensure retained history/replay either preserves the richer definition or
   explains any unavailable evidence instead of claiming an exact replay.
5. [ ] **Qualification and documentation.** Test standalone, inserted and
   overlaid tones, rapid preview replacement, both speech lanes and remote
   delivery. Start listening with spelling, then capital-letter/all-caps and
   indentation cues. Record platform/runtime coverage separately from unit
   results. Update supported protocols, guides and status only as implemented.
6. [ ] **Later patterns, if useful.** Evaluate bounded pitch sweeps and short
   rhythmic sequences using existing timeline facilities where possible.
   Chords, noise, modulation, arbitrary envelope curves and general synthesis
   remain deferred until there is a demonstrated accessibility use case.

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

## Implementation contract and effort handoff

The following is the implementation target, not an advertised server capability.
High is sufficient for the next bounded patches. Reassess only if a regression
exposes a new ordering, cancellation or compatibility conflict.

### Wire and capability boundary

Preserve standalone tone version 1 and timeline versions 1 through 5 unchanged.
Add standalone tone version 2 and timeline version 6, gated by a complete
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

Once a worker negotiates version 6, use it consistently for its structured
timelines, including those with Classic or no tones. Do not alternate versions
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
paths as well as queued standalone tones. Audit the current timeline-render
error handler: it can mark failure and queue dry speech. The cancelled outcome
must bypass that fallback. Tests must cover cancellation during materialization,
not just an already-cancelled request or active playback.

### Client and next patches

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
and completion. This verifies transport and execution, not audible output;
platform timing and listening qualification remain open.

Next implement worker-local client negotiation, explicit old-worker degradation
and individual spelling gain with matching editing feedback and previews.
Individual gain in Emacsvox follows this dependency: action-only editing cues
and previews currently use standalone version 1, which cannot represent gain.

In Emacsvox, carry gain and shape in frozen concrete actions so live playback
and replay use the captured values. Rules validate literal and fact-derived
values before transport. Keep spelling's current Customize options and explicit
Save; named richer rule overrides belong to existing personal-rule storage.
Introduce user-data version 11 only when saving those new rule fields; preserve
older files and versions when no richer rule is written. Old readers must fail
explicitly on the new format, not reinterpret the shape. Check the current schema
number again before implementing because other work may advance it.

Preset values remain provisional listening choices, with no automatic loudness
compensation or default changes. Initial trials can use Soft (10/0/1/15 ms),
Plucked (1/25/0.25/10 ms), and Rounded (10/10/0.7/15 ms), expressed as
attack/decay/sustain/release; sustain is unitless. Classic remains the default.

No new ADR is created by this plan. If the contract work requires changing an
accepted architectural boundary, identify that decision and obtain separate
approval for its ADR before proceeding.
