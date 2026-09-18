# Public engine parameter catalogues, 2026-09-18

This slice connects the read-only `engine_parameter_catalogue_v1` capability to
current-worker metadata. Native registration, ordinary native speech, private
native previews, explanations, routed application evidence and the Emacs editor
remain separate work. `engine_voice_parameters_v1` remains unadvertised.

## Implementation

The version-1 control envelope accepts `get_engine_parameters_v1` and returns
`engine_parameters_v1`, using the same typed query/page shapes as helper 6.
Required nulls, unknown request fields, duplicate keys, identifiers, cursors,
revisions and complete control-message limits are checked. Existing operations
and helper versions keep their earlier shapes. Unsupported engines return
`not_described`; old helpers return `unsupported_helper`. Absent or disabled
engines and excluded voices have explicit unavailable results.

Each server connection admits one catalogue query independently of its speech
command and synthesis queues. An admitted query owns at most one coordinator
and one adapter thread. Its response deadline is one second. Further queries
receive busy, and a timed-out adapter retains admission until it exits. Late
results are discarded; closing the connection suppresses pending responses.
There is no retry queue or growing collection of detached work. The helper's
200 ms transaction limit and separate cleanup ownership remain unchanged.

The engine abstraction and its isolation/eligibility wrappers preserve this
read-only path. Queries do not recover, reconnect, enumerate installed assets,
load native voices, or stop/synthesize speech. An occupied engine or pending
recovery returns busy. A stale first-page revision is checked against the ready
identity without retiring the helper as though it had sent an invalid reply.
Page responses retain helper validation and request/engine/voice correlation.

## Verification

Nine new regressions cover public request fixtures and malformed correlation,
actual helper revision guards followed by ordinary speech, disabled voices and
empty managed providers, bounded admission through a blocked query, discarded
late results, independent connection admission, disconnect, malformed engine
responses and pending recovery. Existing parent query tests also exercise the
new engine-facing method.

The locked Rust workspace passed 922 tests, with one existing long-session
eSpeak stress test ignored. Workspace Clippy (all targets), formatting and
local Markdown link checks passed. Native Windows acceptance and staging
observations follow below.

Full Windows development staging passed as `2043239474542241`, including
checksums, deterministic Windows helper hashes, inventory and live companion
synthesis. The main source diff recorded by staging is
`5cfc4b43d3cec9c1acad45972ee7b5cb454921ad94e9f44de6a0e1e677b3ccf0`;
this report and its observations were appended after that build. The qualified
Windows helper binaries are unchanged from the preceding parent slice.

The [native observations](data/2026-09-18-public-parameter-catalogues.json) cover
all eight Eloquence and nine DECtalk presets through the packaged server's
public control channel, using the null audio backend. They returned eight and
28 controls respectively, with the expected stable catalogue/runtime identity.
Engine-level pages did not claim voice-default readback. Unknown engines and
voices, an undescribed engine, missing required nulls, stale revisions and
expired cursors returned the specified results/errors. Stale metadata guards
left the helper identity unchanged; both subsequent exact ordinary previews
completed with nonempty canonical PCM and the requested physical voice.

During a long Eloquence preview, all twelve concurrent catalogue requests
returned busy. Those replies plus a capabilities reply arrived within about
1.7 ms in this run. This is a control-response observation, not an acoustic
latency measurement. The preview was cancelled and a fresh ordinary preview
completed successfully. The process then logged normal shutdown and helper
retirement. Tests opened no audio device and establish no listening quality.

Final inspection found the development desktop launcher had been restored outside
this slice to `e1ecdb481ee08fd0`. Its new comment records that the active voice
library requires Piper. Both the earlier parent test package and this catalogue
package omit Piper under the existing development staging policy; neither is a
suitable replacement for that active profile without a matching Piper build.
The restored launcher was preserved. No live Emacs process was changed. The
catalogue package remains staged separately; exposing/editing these controls in
Emacs still requires native registration, routing, preview and client work.
