# ADR 0003: Bounded progressive audio and truthful markers

- Status: Accepted
- Consolidated: 2026-09-27 from progressive synthesis, requested-anchor and letter-reserve decisions.
- Related: [Engine isolation](0001-engine-isolation-and-distribution.md),
  [PulseAudio](0005-native-pulseaudio-output.md),
  [voice selection](0006-voice-selection-and-customization.md).

## Context

Transporting chunks only after synthesis completes adds whole-utterance delay
and duplicate buffering. Progressive native callbacks can reduce onset, but
require bounded backpressure, ordered markers and a clear point after which
retry would repeat speech. Chunk counts alone do not describe navigation latency.

## Decision

### Negotiate progressive semantics

Helper protocol 5 permits interleaved bounded PCM and marker frames. Versions
1–4 keep their buffered audio-then-markers behavior. Advertise `streaming_pcm`
only when nonempty audio is emitted while native synthesis is still active;
buffered adapters remain valid. The host checks sequence, frame alignment,
cumulative limits, realized voice, marker order and final frame totals.

Publish a marker before the audio chunk containing its frame; final-boundary
markers may follow the last audio chunk. One stateful sinc converter maps native
mono/stereo PCM and marker clocks continuously into canonical frames. Do not
restart filters at callback boundaries or replace the established quality with
independent linear conversions. Native Rust adapters can emit canonical windows.
Fixed-capacity channels apply backpressure rather than retaining unbounded audio.

Progressive speech is one tracked source. Explicit terminal production defines
completion; temporary lack of PCM does not. The playback-start frame count may
be zero while the final duration is unknown; tracked completion is authoritative.
Consumption events measure the source clock, not physical acoustic output.

### Commit an attempt once and contain failure

Before the first progressive PCM is accepted, routed start metadata and marker
preambles remain transactional and fallback may retry the original request,
within the existing four-attempt limit. After PCM commitment, engine failure
terminates the utterance without
splicing another engine or replaying it. Output-device failure never counts as
an engine failure or authorizes cross-engine replay.

Cancellation closes bounded channels, suppresses stale callbacks, cues and
semantic events, and retains native stop plus the helper watchdog. Replacement
waits for confirmed cleanup. Operations needing the complete waveform may use
the buffered collection path; this does not permit a streaming adapter to buffer
all ordinary speech unnecessarily.

### Keep anchors source-accurate

Requested anchors report their actual exact, word-boundary, span-boundary or
omitted resolution. Insertions shift later output frames; overlays and effect
tails carry across bounded windows. Unsupported anchor routes remain buffered.
Generated text/commands must preserve mapping to the original UTF-8 source.

TGSpeechBox reports exact caller-requested anchors through frontend user indexes,
not invented word/sentence/phoneme timing. Split native input only at requested
source boundaries, preserve punctuation and use continuation handling; ordinary
unanchored speech remains one frontend call. Indexes increase across utterances
and the drained native owner is renewed before exhaustion. Publish the native
index convention's boundary before subsequent audio, retaining at most its
one-native-frame resolution error. Empty text segments and shared/start/end
boundaries resolve truthfully. Other engines qualify their own native mappings.

### Bound the initial playback reserve

Real-device progressive speech normally starts after three nonempty windows,
or the terminal for a shorter source. Cue-only updates accompany the next PCM
or terminal and cannot consume the reserve. Null output attaches immediately.

Only explicit letter-navigation requests (`l` and its negotiated palette-aware
`emacsvox_letter` form) may also start after 40 ms of canonical rendered PCM is ready. Retain the three-window and terminal release
conditions so small windows and short streams make progress. This is a frame
threshold, not a timer, text-length heuristic or configurable native setting.
Select it before publishing PCM. Other speech, timelines, previews and buffered
engines retain their existing behavior; character-rate and uppercase handling
are unchanged. PulseAudio's separate native priming remains independent.

Changing the reserve requires retained device evidence covering onset, producer
stalls, final consumption, rapid replacement and recovery. A smaller reserve
does not prove gap-free acoustic output or eliminate native cleanup latency.

## Consequences and alternatives

Early playback reduces latency while introducing partial-output failure and
backpressure obligations. Tests cover blocked consumers, cancellation phases,
late/out-of-order markers, truncated streams, old versions, exact frame counts
and conversion independent of callback boundaries. Preserve the retained
[performance and native evidence](../benchmarks/README.md).

Treating old transport chunks as progressive silently changes their contract.
Queueing one playback source per wire frame exposes transport boundaries as
completion boundaries. Proportional or IPA-derived markers invent precision
without a source mapping. None is an acceptable substitute.

Exact wire shapes and bounds remain in the
[helper](../protocols/helper.md),
[control](../protocols/control.md) and
[timeline](../protocols/presentation-timeline.md) references.
