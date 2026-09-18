# Public native voice registration

The existing control envelope now accepts `register_logical_voices_v3` and
returns `logical_voices_registered_v3`. This connects the
[internal admission contract](2026-09-18-native-choice-admission.md) to the
[connection-owned catalogue cache](2026-09-18-parameter-catalogue-cache.md).
The complete native execution capability remains unadvertised.

## Behavior

A complete replacement may mix legacy, layered and engine-layered definitions
within the existing registry generation domain. New envelopes reject duplicate
keys, unknown fields, missing required nullable fields and native data inside
older definition modes. Native registration requires a positive request ID.
Known-invalid native values reject the replacement without changing the registry.
Unknown schemas and absent runtimes retain their inert saved data.

The command dispatcher snapshots only this connection's current cached
catalogues. It performs no discovery, synthesis, helper recovery or voice loading
to satisfy registration. Missing metadata yields deferred status; absent or
administratively disabled engines yield unavailable status. Qualified controls
yield supported status. An identical-generation retry can refresh these statuses
without rewriting saved settings. A new connection starts with its own empty
cache and registry. Ordinary speech can use the engine while registration uses
its already cached controls.

Admission operates on a private candidate. Current workstation policy controls
resolution and disablement, and the acknowledgement reports one native status
for each non-null native block. Only after the entire public reply fits the
control limit does the candidate replace the live registry. Stale generations,
conflicting replacements, malformed requests, invalid values and overlarge
acknowledgements preserve the existing definitions.

Old registration operations retain their grammars and one shared generation
domain. Older speech/timeline paths still reject engine-layered definitions
instead of silently dropping their native settings. A supported registration
acknowledgement describes validation, not native application or actual playback.

## Automated verification

Eight new regressions cover paired request/acknowledgement fixtures, strict
malformed-message handling and correlation, mixed definitions, same-generation
status refresh, atomic value/generation failures, administrative policy,
connection isolation and runtime cache replacement. The reply-bound test uses a
body that fits the limit but exceeds it once the public envelope is included;
publication is rejected. Existing old-speech guards now begin with public v3
registration.

The locked workspace passed 977 distinct tests plus one child-process rerun,
with two ordinary skips. Workspace Clippy across all targets, pinned formatting
and whitespace checks passed. The retained qualified-runtime probe also passed
separately using the unchanged Eloquence and DECtalk helpers:

```sh
OMNIVOX_NATIVE_ROUTING_CASES=/path/to/cases.json cargo test --locked -p omnivox-cli qualified_helpers_route_native_choices_without_playback -- --ignored --nocapture
```

Its configuration is described in the
[routing report](2026-09-18-native-routed-execution.md). Public registration uses
the real cached controls before internal fallback and exact private selection
produce native PCM with the requested parameter readback. These Linux-parent
checks open no audio device and do not establish listening quality or actual
playback.

## Packaged Windows verification

Full Windows development staging passed as `79005b934881d167`, including
checksums, deterministic helper builds and live companion synthesis. The
recorded Omnivox tracked-diff hash is
`b18132c7f21e6fd7a47d3a619be97fbca8228aa7179cefce3b523fcab51e6479`.
Final report observations were appended after staging. The
[retained observations](data/2026-09-18-public-native-registration.json) include
both parent paths and the staged executable identity.

The packaged Windows server returned deferred status before catalogue discovery
and supported status after caching all eight Eloquence and 28 DECtalk controls.
Identical-generation retries refreshed status. Invalid values and a public
acknowledgement exceeding the size bound left the previous generation intact.
A separate connection reported deferred status independently. Administrative
disablement made DECtalk settings unavailable while preserving them.

During an ordinary Eloquence preview, a live catalogue query returned busy while
native registration used its cached metadata and completed in about 3.1 ms in
this run. Cancellation and an ordinary follow-up preview completed successfully.
This is a control-response observation with null audio output, not an acoustic
latency or listening-quality measurement. Public native speech is not implemented
by this registration slice.

The test payload is staged separately below
`servers/omnivox-bin/native-registration-check` in Emacsvox. The ordinary full
development target omits Piper, so it was not selected for the user's active
voice-library profile. No desktop launcher or live Emacs session was changed.
Documentation links and the Emacsvox documentation release check passed.

## Remaining work

Connect timeline 5 and marker 4, preserving native application evidence through
accepted and actually consumed audio. Add strict native preview and explanation
operations, then the Emacs parameter editor and two-lane client acceptance.
The registration operation alone does not make native settings usable from Emacs.
