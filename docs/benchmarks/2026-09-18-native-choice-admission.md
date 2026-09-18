# Native choice admission and preparation, 2026-09-18

This slice follows [shared native synthesis execution](2026-09-18-native-synthesis-execution.md).
It implements the internal registry and preparation boundary from
[the accepted contract](../engine-voice-parameters.md). Public registration,
native timelines, routed execution/evidence, private previews and the Emacs
parameter editor remain subsequent work. No public native capability is enabled.

## Behavior

Engine-layered definitions retain independent choice IDs, common adjustments and
nullable sparse native blocks. The new body reader is bounded and rejects
unknown fields, duplicate keys, malformed operations, engine mismatches and
nonfinite values. Old registration readers do not accept the new mode.

Registration uses the existing generation domain and publishes a candidate only
after validating the whole replacement and its bounded status summary. Invalid
known parameters, values and scopes leave all previous state intact. Missing
engines and unknown schemas remain saved but unavailable; absent or busy
metadata is deferred. A retry cannot erase native data through an older API.
Native status is provisional metadata, not proof of native application.

The caller supplies a borrowed immutable snapshot of current inventory and
complete catalogues from one connection. Registration does not fetch catalogues,
load engines, wait on native calls or reconnect workers. Snapshots reject
conflicting generations and duplicate metadata. The eventual public handler
must also bound the complete response envelope before publishing the candidate.

Preparation starts from the original common style, actual choice occurrence and
context. Different choices for the same physical voice retain different native
settings. Portable selectors are requalified against the actual voice. Policy
fallback has no choice-native block. Explicit zero/default operations and
context presence survive; common mapping formulas remain adapter-owned.

Each prepared native block freezes the current runtime identity. Metadata refresh
does not change the saved definition or registry generation. Strict preparation
rejects an unavailable block; explicitly permitted common-only preparation keeps
the common style and records a reason without manufacturing application evidence.
Older ordinary routing and timeline versions 1–4 reject engine-layered definitions
so settings cannot silently disappear during the remaining integration.

## Verification

Twelve new regressions cover atomic replacement, retry conflicts, missing engines,
unknown schemas, strict decoding and bounds, nonfinite legacy values, actual
choice/context composition, explicit degradation, metadata refresh, voice-specific
validation and older-path rejection. The locked workspace passes 945 distinct
tests plus one child-process rerun, with one existing stress test ignored.
Workspace Clippy across all targets, formatting and whitespace checks pass.
The final documentation checks cover repository-local links and Emacsvox's
release documentation gate.

The retained [silent helper probe](../../omnivox-tts/examples/helper_parameters_probe.rs)
now registers a named choice against a current catalogue, resolves and prepares
it, then submits its native block through the shared engine interface and voice
eligibility wrapper. Both qualified Windows Eloquence and DECtalk helpers return
supported registration status and nonempty buffered/progressive PCM. Applied
explanations report the requested native value. Stale strict requests fail;
explicit common-only requests and fresh-identity recovery succeed.
[Recorded observations](data/2026-09-18-native-choice-admission.json) include the
helper hashes and the selected native readback.

As in the preceding Linux-parent probe, cancellation at first PCM retired each
helper for both native and ordinary speech; fresh connections recovered. These
checks do not open an audio device or establish listening quality, acoustic stop
latency, public routed fallback or Windows-parent acceptance of this new slice.

## Remaining integration

Connect current-connection metadata ownership to public registration, then native
timelines and actual speech attempts. Propagate native receipts through committed
PCM and playback evidence, wire strict private previews and explanation lookups,
and expose the engine-described controls in Emacs. The complete native capability
must remain unadvertised until these paths work together.

No Windows package was rebuilt or selected for this internal-only slice. It uses
the unchanged qualified helpers from the preceding staged runtime. Full Windows
development staging remains required when public transport is connected. No live
Emacs session or desktop launcher was changed.
