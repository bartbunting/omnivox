# ADR 0006: Bounded Progressive Synthesis

- Status: Accepted
- Date: 2026-09-03

## Context

Helper protocol versions 1 through 4 divide completed PCM into bounded wire
frames, but do not provide progressive synthesis. Each adapter first retains
the complete native waveform, the helper host then emits `audio_chunk` frames,
and the main process reassembles every frame before it queues playback. This
keeps the transport bounded per frame but adds the whole synthesis duration to
time-to-first-audio and retains multiple copies of longer utterances.

TGSpeechBox, Eloquence, and DECtalk can all produce PCM incrementally. Their
existing process boundaries, marker support, cancellation behavior, and
runtime-supply policy remain governed by ADRs 0001 and 0005. A progressive path
must not weaken those boundaries or let a stalled producer allocate without a
limit. It must also preserve buffered engines and old helpers.

Once a progressive result has supplied audible PCM, ordinary retry through a
different voice would repeat or splice speech. Marker-capable helpers also
need to publish a marker before playback can pass its frame, rather than
sending every marker after all PCM as versions 1 through 4 require.

## Decision

Helper protocol version 5 adds progressive synthesis semantics without adding
an unbounded payload or a second transport:

- The JSON envelope and existing `synthesis_started`, `audio_chunk`, `markers`,
  and terminal response shapes remain unchanged. Version 5 permits `markers`
  batches and audio chunks to interleave. A helper must publish a marker before
  publishing the chunk containing that marker's frame; marker offsets therefore
  cannot be behind the number of frames already published. Markers at the final
  frame boundary may follow the last audio chunk.
- A helper advertises `streaming_pcm` only when its native adapter begins
  emitting non-empty chunks while synthesis is still active. Buffered adapters
  continue to advertise `buffered_pcm` and remain valid version 5 peers.
  Versions 1 through 4 retain their original audio-then-markers ordering and
  buffered capability requirement.
- Omnivox accepts a progressive helper only after negotiating version 5. It
  validates chunk sequence, frame alignment, cumulative PCM and marker limits,
  marker ordering, realized voice, and the terminal frame count in the PCM
  format announced by `synthesis_started`. Progressive signed 16-bit PCM may
  be mono or stereo at a supported native sample rate. One bounded, stateful
  sinc converter maps its audio, markers, and terminal count into canonical
  Omnivox frames without restarting the filter at wire-chunk boundaries. The
  existing full-result API collects the canonical stream for callers that
  require buffering.
- The engine abstraction gains an opt-in progressive method with a buffered
  default. Rust native adapters emit canonical Omnivox PCM through that method;
  a helper protocol peer may instead describe and emit its native mono/stereo
  format so the receiver can apply the same maintained converter used by other
  adapters. Cross-thread and playback channels have fixed capacities;
  backpressure stops the producer rather than growing memory.
- Playback can start before the final frame count is known. In the existing
  playback-marker version 2 envelope, `utterance_started.frame_count` is zero
  for such a non-empty progressive source; a positive value continues to mean
  the exact completed count. The tracked terminal event remains authoritative
  for completion or cancellation.
- A real-device playback source is attached after three non-empty PCM windows
  have filled its fixed-capacity channel, or after the terminal message for a
  shorter source. Cue-only updates are coalesced into the next PCM or terminal
  message so they cannot consume this bounded audio reserve. The null backend
  attaches immediately because it has no device clock or underrun risk.
- Runtime fallback is allowed only before the first progressive PCM chunk has
  been accepted. Routed start metadata and marker/anchor preambles remain
  transactional until that commitment, so a failed attempt cannot contaminate
  a clean fallback. An error after that point terminates the utterance and
  affects health normally, but does not splice in another engine. Cancellation
  closes the bounded channels, stops the native engine, and retains the existing
  helper watchdog as the hard containment boundary.
- Version 5 `requested_anchor` markers may name their actual `exact`,
  `word_boundary`, `span_boundary`, or `omitted` resolution; absence retains
  the older exact meaning. Supported anchors drive bounded incremental timeline
  rendering. Insertions shift later marker/event frames, overlays carry between
  windows, and resolution plus semantic events are published on the same
  progressive playback clock. Unsupported anchor routes stay buffered.
- Operations that need future knowledge of the complete waveform may collect
  the progressive stream and use the established buffered path. This is a
  per-request safety fallback, not permission for a helper advertising
  `streaming_pcm` to retain every ordinary request.

TGSpeechBox is the first progressive adapter because its pull API already
returns bounded 44.1 kHz mono blocks and it has no markers. Eloquence and
DECtalk follow through the common Windows helper host, sending their native
11.025 kHz mono callback blocks and marker clock for continuous conversion by
the receiver. Other callback-based helpers can adopt the same contract
independently after equivalent tests.

## Consequences

Ordinary speech can reach the mixer after the first bounded native block rather
than after the complete utterance. Longer utterances should show the largest
dispatch-to-source improvement. Short inputs may be dominated by phonemization,
process communication, and queue setup.

Progressive playback introduces explicit backpressure and a partial-output
failure state. Tests must cover a blocked consumer, cancellation at each phase,
late or out-of-order markers, truncated streams, terminal count mismatches, and
old-version interoperability. Stateful conversion must also prove that output
does not depend on native callback boundaries, completes at the exact scaled
frame count, and does not replace the established sinc quality with independent
or linear per-chunk conversion. Benchmarks must identify the null backend and
must not describe first-source timing as physical acoustic onset.

The progressive path does not require a new dependency, remove the buffered
API, change companion provenance, or move a native runtime into the main
process. Effects or presentation operations that cannot preserve their
semantics across windows continue through buffered collection until they gain
a tested incremental implementation.

## Alternatives considered

### Treat version 4 transport chunks as progressive

Rejected. Existing version 4 helpers legally send markers only after all audio
and advertise buffered PCM. Changing those semantics in place would make
capability validation and marker ordering ambiguous.

### Queue each wire chunk as a separate playback source

Rejected. It would expose chunk boundaries as completion boundaries, complicate
continuous cancellation and marker timing, and risk gaps between sources.

### Buffer only marker-capable engines

Rejected as the final design. It would leave Eloquence and DECtalk unable to use
their native callback latency even though their markers can be ordered ahead of
the corresponding audio. Buffered fallback remains appropriate only for
specific transforms that need the final waveform.
