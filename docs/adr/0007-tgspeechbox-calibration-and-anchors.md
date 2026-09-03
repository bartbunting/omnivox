# ADR 0007: TGSpeechBox Calibration and Requested Anchors

- Status: Accepted
- Date: 2026-09-03

## Context

ADR 0005 introduced TGSpeechBox with an explicitly provisional rate mapping
and no marker claims. The exact pinned upstream revision also contains an
index-aware DSP pull that was not used by the initial adapter. It stops at a
frontend user-index transition without changing the concatenated PCM stream.
The frontend can assign one opaque index to all frames produced by one IPA
queue call, but it does not map generated word, sentence, or phoneme frames
back to original UTF-8 source ranges.

Emacsvox capitalization tones and presentation actions already request opaque
source boundaries through the common synthesis-anchor contract. They do not
require TGSpeechBox to invent a general linguistic marker stream. The initial
TGSpeechBox rate curve also produced large avoidable differences from the
Eloquence compatibility reference governed by ADR 0004.

## Decision

- TGSpeechBox advertises exact requested-anchor support, while word, sentence,
  phoneme, and unrequested native-index marker capabilities remain false.
- For an anchored request, the helper groups requested positions by original
  UTF-8 byte offset and divides the source only at those boundaries before text
  preparation and eSpeak phonemization. The following non-empty frontend
  segment receives an opaque user index. Index values increase across
  utterances because the persistent DSP retains its last reached index; the
  helper recreates the drained player before integer exhaustion. Anchors
  separated only by text that produces no audio resolve to the same PCM
  boundary. Start and trailing anchors resolve to frame zero and the final
  frame respectively.
- Intermediate source segments with no terminal punctuation use TGSpeechBox's
  continuation clause type. Explicit punctuation is preserved. An unanchored
  request remains one frontend call, so ordinary speech does not acquire
  artificial segmentation.
- The adapter uses `speechPlayer_synthesize2` for every bounded pull. When it
  reports an index, the returned PCM through the marker frame's first silent
  sample is published first and the anchor is placed at the immediately
  following frame boundary. This is the upstream callback convention, is at
  most one 44.1 kHz sample after the native transition, and keeps protocol-v5
  metadata ahead of all subsequent audio. Skipped native indexes within one
  time-stretched tick resolve together, as no distinct output frame exists
  between them.
- The buffered 22.05 kHz comparison path scales native anchor frames into the
  canonical 44.1 kHz result and remains buffered. The default 44.1 kHz path
  interleaves anchors with progressive PCM.
- Adam `en-us` defines one monotonic piecewise-linear rate table measured with
  the standard 22-word English corpus. It follows Eloquence `v1` through host
  rate `1.0` and saturates honestly at TGSpeechBox's native `4x` ceiling from
  host rate `1.2` through `2.0`. The retained baseline, reference, and
  three-repetition calibrated reports are linked from the speech-rate guide.
- The existing helper protocol v5 and synthesis contracts are sufficient. No
  public protocol field, native extension, dependency, or process boundary is
  added.

## Consequences

TGSpeechBox can keep capitalization tones and timed presentation actions on
the bounded progressive path. Callers receive exact results for only the
positions they requested; they still cannot ask TGSpeechBox for a truthful
general word or sentence timeline.

Anchored synthesis can have a small frontend continuation gap at a requested
boundary, and a boundary inside a word necessarily makes eSpeak phonemize the
two source pieces independently. Presentation compilers should continue to
place actions at natural UTF-8 boundaries when possible. These effects are
preferable to claiming offsets that the native frontend cannot substantiate.

At ordinary rates the Adam calibration closely tracks the Eloquence reference.
Individual profiles and languages can differ, and rates above `1.0` still show
the engine's real ceiling. TGSpeechBox remains experimental because its pinned
upstream line is beta and Windows x64 GNU remains the only runtime-accepted
distribution target; calibration and requested-anchor support do not change
those independent qualifications.

## Alternatives considered

### Infer general markers from generated IPA or frame counts

Rejected. Text preparation, pronunciation dictionaries, Unicode normalization,
and eSpeak phonemization can all change length. A proportional or token-count
mapping would look precise without preserving original source offsets.

### Keep marker-dependent TGSpeechBox requests buffered

Rejected. The selected upstream DSP already supplies the index boundary needed
by the established protocol-v5 ordering contract, so buffering would add
latency without improving truthfulness.

### Force the Eloquence curve above TGSpeechBox's native ceiling

Rejected. Post-synthesis time compression would reduce intelligibility and
complicate marker timing. ADR 0004 requires honest monotonic saturation.
