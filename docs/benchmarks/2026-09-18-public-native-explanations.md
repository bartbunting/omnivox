# Public native parameter explanations

The public `explain_voice_parameters_v1` / `voice_parameters_explained_v1` pair
now connects resolved private choices and retained playback references to the
existing helper-6 explanation operation. Catalogue and explanation reads share
one connection-owned admission slot and a one-second response deadline. The
adapter retains that slot until it exits even if the deadline already reported
unavailability. The engine-isolation wrapper forwards idle explanations and returns
busy during active synthesis or pending recovery without initiating recovery. Connection closure suppresses late replies.

Drafts reuse strict native preview validation, selected-choice routing and
composition without submitting text or synthesis. They preserve shared settings,
choice adjustments, context masking, placement and rate guards. Selection cannot
substitute another choice. Complete private and applied engine exclusions remain
in force. A no-native draft may still describe the engine's common mapping.
The result is a prediction without repertoire admission or playback evidence.

Applied lookup uses the same bounded reference owner as timeline and preview
playback. Each reference now retains the original choice alongside physical voice,
helper-local plan, catalogue identity, weak engine owner and worker epoch. Queries
use that recorded identity rather than current palette contents. Unknown or
evicted references and changed workers report expiry without synthesizing again.
Helper correlation and parent-side validation reject changed voices, plans,
identities, runtime epochs and unsupported readback claims. The public result
keeps the connection-owned plan ID rather than leaking the helper-local ID.

Wire readers reject missing, unknown and duplicate members. Drafts require a
selected choice and nullable expected rate; no text is accepted. Ready evidence
retains the existing 64-row limit and typed parameter provenance. Full response
encoding remains subject to the bounded control envelope.

## Verification

New regression coverage exercises paired wire fixtures, strict requests and
responses, planned/readback separation, exact duplicate-voice selection, native
values and context preservation, missing metadata, disabled choices, rate guards,
no-native drafts, connection isolation, reference eviction, worker changes,
mismatched applied evidence, deadline admission and suppression after closure.
The query-service test also uses the production isolation wrapper to verify
forwarding and refusal to recover on a metadata query.
The existing helper integration test also reaches applied explanation through
the engine trait used by the public server. Fake engines panic if queries attempt
synthesis, recovery, stopping, voice enumeration or loading.

The native capability bundle remains unadvertised. Emacs editing, persistence,
comparison lifecycle and main/notification client acceptance remain separate work.

The locked workspace passed 1012 distinct tests plus one child-process rerun,
with two expected skips. Workspace Clippy with warnings denied, formatting,
whitespace and documentation-link checks passed.

## Packaged Windows acceptance

Completed on 2026-09-19 AEST. Full Windows development staging passed as
`460c4fbb8b1d85da`, based on Omnivox `5270daa` plus
tracked-diff SHA-256
`b5549f139f4905a5bbf13e19dad88b2bd7984f343c72c813d3c4c487fb93923b`.
Final observations and this verification section were added after staging; the
tested executable source did not change. The separate package is under Emacsvox
`servers/omnivox-bin/native-explanation-check`. Full companion checks passed;
this development package omits Piper. It was not selected for the active
voice-library profile. No desktop launcher or live Emacs session changed.

The [retained observations](data/2026-09-18-public-native-explanations.json)
include complete build provenance, 24 control replies and ordinary timeline
markers. Packaged DECtalk and Eloquence passed planned versus applied values,
verified native readback, distinct tuning for the same voice, preservation of
historical values after later drafts and playback, explicit equal-value context
masking, common-mapping explanations without native edits, missing metadata,
unknown/other-connection plan expiry, and shared timeline/preview lookup.

During active native synthesis the explanation query returned busy in about
2.8 ms in this run. Cancellation and later ordinary speech
and explanations succeeded. The old Eloquence plan remained available after
this cancellation; runtime replacement and eviction are covered by deterministic
regressions rather than claimed from this particular cancellation run.
The native capability bundle remains unadvertised pending final client acceptance.

These checks use null audio output and establish protocol behavior, source
consumption and engine readback. They do not establish acoustic quality,
physical-device playback or Emacs workbench acceptance.
