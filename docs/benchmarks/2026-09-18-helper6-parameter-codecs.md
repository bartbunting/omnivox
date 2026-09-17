# Reserved helper-6 parameter codecs, 2026-09-18

This follows the [legacy wire boundary](2026-09-18-helper-native-wire-boundary.md)
and implements the next part of the [accepted contract](../engine-voice-parameters.md).
It adds no advertised capability or live helper operation.

## Implementation

The Rust codec covers the new catalogue and explanation operations and the
extended synthesis request/start response. Required nullable members cannot be
omitted. Native values retain explicit zero, false, default and omission, and
context retains explicit dimensions independently of their numerical values.
Unknown fields, duplicate decoded keys, malformed identities and invalid bounds
are rejected. Common settings, text and anchors reuse existing validation.
The boolean descriptor uses an empty struct variant so Serde rejects extra
members; its JSON representation remains unchanged.

Response correlation checks the submitted request ID, helper engine, voice,
expected applied identity and requested masked IDs. A strict native request
cannot accept a common-only start. Planned explanations cannot claim native
readback; applied explanations must identify their retained plan. These checks
validate messages, not proof that the native calls ran or audio was heard.

Catalogue pages contain at most 64 descriptors and respect the 256 KiB metadata
message bound. Assembly keeps one engine/voice/runtime identity and mapping set,
rejects repeated cursors and duplicate IDs, and bounds the complete inventory to
512 descriptors. References to other pages are checked on completion. Invalid,
stale or operationally busy pages do not change the accepted assembly state.
The query owner remains responsible for request correlation and deadlines.

## Verification

The independent contract fixtures round-trip through the codecs. Twelve focused
tests cover omitted/null fields, nested/escaped duplicates, unknown and mixed
forms, scalar/default preservation, context identity, strict/common-only receipt
correlation, planned/readback distinction, page/runtime consistency, atomic
rejection, cross-page references and both byte/count bounds. A 512-row inventory
assembles successfully while an additional descriptor is rejected.

Locked workspace tests passed: 900 tests, with one existing long-session eSpeak
stress test ignored. The complete suite and all 12 codec tests passed on the
final source, including the identifier grammar for plan IDs (distinct from
opaque page cursors). Workspace Clippy passed with Rust 1.97.1. Formatting,
diff and local documentation-link checks passed.

An initial Windows staging attempt exhausted the 16 GiB temporary filesystem.
Only generated compiler outputs from two completed temporary verification runs
were removed; source snapshots, reports and staged runtimes were retained. The
resumed full target passed, including live companion checks. Final full staging
of the corrected source passed as build `b1b6060ef0dec0ab`, including checksums,
live inventories and Flite, RuTTS and TGSpeechBox synthesis. This development
payload omits Piper under the existing staging policy.

The final staged Flite, Eloquence and DECtalk helpers each passed three isolated
compatibility probes: version-6-only greeting rejection, selection of version 5
from a list also containing 6, and rejection of a reserved operation on a live
version-5 session. Rejected requests produced no start or PCM frames. Rust retains
its existing malformed-frame session retirement; Windows retains its recoverable
request loop. [Retained negotiation evidence](data/2026-09-18-helper6-negotiation.json)
records the exact binary hashes and response envelopes.

Both Windows helper hashes match the previous slice's streaming/cancellation
binaries; their native adapters were not changed or requalified in this slice.
Both repositories' documentation gates passed. Staging used a separate runtime
root and a self-contained source snapshot with development provenance. The
desktop launcher remains on `e1ecdb481ee08fd0`; no live Emacs was restarted.
These checks do not establish helper-6 execution or acoustic quality.

## Remaining integration

Live negotiation remains on versions 1–5. The separate reserved codec does not
accept hello, ordinary PCM, markers or terminal frames; the existing legacy
readers still own the live dispatcher. The helper-6 dispatcher must compose
these operation codecs with shared stream frames only after native handlers are
ready. Engine-owned catalogue construction, native request execution, bounded
applied-plan retention, query scheduling and receipt-before-PCM ordering remain
pending, followed by public transport and Emacs editing.
