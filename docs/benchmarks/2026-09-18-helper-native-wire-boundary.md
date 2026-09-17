# Native parameter helper wire boundary, 2026-09-18

This is a compatibility prerequisite for [helper 6](../engine-voice-parameters.md),
following the internal [DECtalk execution slice](2026-09-18-dectalk-native-execution.md).
It does not advertise helper 6 or expose the native editor.

## Failure and change

The existing Rust flattened helper envelopes allowed unrecognized fields to be
ignored. A protocol-5 synthesis request carrying `voice_parameters` could thus
be treated as ordinary common-only speech without reporting the lost settings.
The Windows host already rejected unknown request fields, but its
`JavaScriptSerializer` overwrote duplicate members before validation.

Rust envelope readers now retain a duplicate-free JSON tree, remove only the
envelope fields, and decode strict body/settings/PCM/marker types. Empty tagged
operations receive an explicit member check: Serde's internally tagged unit
variants otherwise ignore extras even with `deny_unknown_fields`. Requested
anchors and version-specific fields are checked at the wire boundary. Duplicate
names compare after escape decoding at every depth. Shared discovery descriptor
schemas remain unchanged; their members are still subject to duplicate checks.

The Windows host validates bounded JSON syntax and member uniqueness before
materializing dictionaries or trusting a request ID. This also rejects
JavaScript-only forms such as unquoted or single-quoted keys, malformed numbers,
invalid escapes and trailing commas. Quoted speech text containing braces,
commas or repeated words is unaffected. Validation remains separate from native
calls, and the protocol loop can accept valid speech after rejecting bad input.

Versions 1–5 retain their framing, serialization, settings and negotiated
capabilities. Earlier versions reject newer fields even when null. Unowned
errors still accept an omitted or null request ID. Protocol 6 remains reserved;
there is no new operation or capability advertisement in this slice.

## Verification

The helper codec suite includes raw malformed input and successful round trips
for all five versions, native-field rejection, nested/escaped duplicate keys,
version-only null fields, valid escaping, unowned errors and frame limits.
The Rust host test proves an unknown native block never reaches synthesis.
All 25 helper codec tests and the host rejection test passed. Locked workspace
tests and Clippy for all workspace targets passed with Rust 1.97.1.

The actual Windows host passes 37 deterministic cancellation cases plus 110
wire validation/recovery cases without a native DLL. The latter assert zero
native calls for invalid input and exactly one call for valid follow-up speech.
The same suite against the previous host passes the 37 cancellation cases but
fails on the first duplicate-key case: it does not return the required unowned
error. This is a regression check against the actual previous source.

The rebuilt pinned-compiler helpers also passed 50 ordinary progressive requests
and 50 cancellation/recovery probes each, with markers, anchors, all six common
controls and health pings. Retained reports:

- [Eloquence streaming and cancellation](data/2026-09-18-helper-wire-eloquence-stream.json).
- [DECtalk streaming and cancellation](data/2026-09-18-helper-wire-dectalk-stream.json).

A direct probe of the previously staged Flite helper reproduced the Rust issue:
a protocol-5 request carrying `voice_parameters:null` produced `synthesis_started`,
PCM and `synthesis_completed` instead of rejecting the unknown member. The rebuilt
Flite helper returns only an unowned `invalid_request` error, before any start
or audio frame. The Rust host retains its existing policy of ending a malformed
input session; the Windows host retains its recoverable request loop.

Full Windows development staging passed for build `1a7bd0803c404f6c`, including
checksums, live inventory and companion synthesis. Both Windows helper hashes
match the binaries used for the native streaming checks above. Missing-runtime
startup and DECtalk discovery checks passed, as did the 12 build-script tests.

The rebuilt main server also synthesized ordinary Eloquence `v1` and DECtalk
`paul` speech through these helpers, producing non-silent complete float WAV
files. This checks the Rust response decoder against actual Windows output.
The first capture timed out after 60 seconds while writing across the WSL UNC
filesystem boundary; repeating with Windows-local output passed. The existing
WAV writer makes individual sample writes; it was not changed in this slice.
[Combined acceptance evidence](data/2026-09-18-helper-native-wire-boundary.json)
retains the Flite before/after frames, helper hashes and server WAV measurements.

The build used a self-contained dirty source snapshot, including concurrent
RHVoice work recorded by development provenance. It is not a clean release.
The desktop launcher remains on `e1ecdb481ee08fd0`; no live Emacs session was
restarted. Tests captured audio silently and do not establish acoustic quality.

## Remaining work

Helper 6 catalogue/explanation operations, identity validation, native execution
receipts and version negotiation remain pending, followed by public transport
and the Emacs editor. This slice does not qualify additional vendor runtimes or
change the internal parameter setters qualified in the preceding slices.
