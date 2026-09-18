# Strict native private previews

The public `preview_voice_v3` / `preview_voice_completed_v3` pair connects
private native choices to the existing preview queue, synthesis executor and
playback tracker. Earlier preview formats retain their wire shapes and meanings.
The full native feature bundle remains unadvertised pending explanations and
final integration acceptance.

Admission validates the complete private draft, freezes the host rate, context,
placement, disabled-engine union and connection-owned catalogue snapshots, and
counts native data in the queued routing payload. It neither replaces the live
registry nor activates voices. Selected-choice requests keep the selected row's
original identity, including two differently tuned rows for the same voice;
there is no selector substitution. Automatic requests retain ordinary synthesis
fallback before PCM commitment and recompose the actual choice independently.

Native execution always uses the helper's require policy. Missing metadata,
unsupported controls and invalid native evidence cannot produce a successful
common-only native preview. Preparation rejection retains a bounded explanatory
message. A deliberately no-native choice is valid and reports required null
application. Failures after accepted PCM retain evidence without cross-engine
replay. Empty audio and stale queued work do not invent started output.

The producer validates the complete native identity before PCM acceptance and
publishes its helper-local plan under the connection's bounded reference owner.
The existing first-frame observation keeps accepted audio separate from actual
source consumption, without formatting, allocation or helper access in the
callback. Distinct applied plans remain separate observations. At most 32
accepted identities are retained; terminal formatting can shorten that list
further to fit the 256 KiB control limit while preserving the independent
last-started identity, frozen metadata and explicit truncation. The combined
choice/native identity has a 48 KiB bound. New response readers reject duplicate
keys and malformed application evidence.

## Automated verification

Twelve new regressions cover paired request/response fixtures, strict nested
request fields and required receipt members, duplicate replies, native evidence
validation, old-format rejection, rate/text/choice bounds, UTF-8 and terminal
truncation, all four buffered/streaming fallback combinations, exact duplicate
voice selection, no-native auditions, metadata absence, invalid helper receipts,
empty audio, stale work, post-PCM failure, private policy freezing and independent
accepted/started plan history. The playback checks use null output and establish
source consumption rather than acoustic quality.

Draft/applied explanations, the Emacs editor and persistence, comparison preflight
on both halves, and main/notification client acceptance remain separate slices.

The locked workspace passed 1000 distinct tests plus one child-process rerun,
with two expected skips. Workspace Clippy with warnings denied, formatting,
diff whitespace and documentation-link checks passed.

## Packaged Windows verification

Full Windows development staging passed as `25169b2408d02bce`, based on
Omnivox `9281f3d` plus tracked-diff SHA-256
`d79948d2f81731823dfcc1220e3f0882ee3c4ab4a47ac44beec8dd8ebca67a47`.
Final observations and this report's verification section were added after
staging; the tested executable source did not change. The separate package is
under Emacsvox `servers/omnivox-bin/native-preview-check`. Full companion checks
passed; this development package omits Piper. It was not selected for the active
voice-library profile, and no desktop launcher or live Emacs session changed.

The [retained observations](data/2026-09-18-native-previews.json) include build
provenance and eleven native-preview or admission replies. The packaged server
passed DECtalk and Eloquence checks for strict rejection without cached metadata,
selected duplicate-voice choices, distinct accepted/started plan references,
explicit equal-context masking, preservation of the named live registry,
comparison-rate rejection, private disabled-engine policy, deliberate no-native
requests, and cancellation followed by successful ordinary and native previews.
The complete native capability bundle remains unadvertised.

These checks used null audio output and establish synthesis, source consumption
and terminal evidence. They do not establish acoustic quality, physical-device
playback, or Emacs workbench acceptance.
