# Orca speech backend for Omnivox

Status: in progress; implementation requested on 2026-09-29. The adapter remains
on Orca branch `feat/omnivox-backend`. Omnivox's `feat/orca-backend` work is merged
into main; helper write/cancellation hardening is unreleased, while its eSpeak
marker-order fix already shipped in 1.16.1.
Reviewed 2026-10-23 against Orca `93d2272bb` and its development guide.
Upstream acceptance and release publication remain separate milestones.
Scope: a direct Python speech backend for the current Orca checkout on Linux,
using an independently installed Omnivox speech process.
Related: [roadmap](../ROADMAP.md#feature-backlog),
[architecture](../ARCHITECTURE.md), [decision index](../adr/README.md).

The initial adapter uses ordered timelines, bounded asynchronous transport,
lazy voice registration and context-level Say All progress. A separate POSIX
supervisor now retires the worker group when Orca's lifetime pipe closes.

## Current delivery state

The feature branch includes backend registration, Meson installation, voice and
style mapping, bounded asynchronous transport, stop/restart and the supervisor.
Orca's `docs/omnivox.md` at commit `93d2272bb` in the
`orca-omnivox-hardening` checkout records a staged GTK smoke test and 36 focused
tests with a real null-output worker, including abrupt parent death,
300 Unicode Say All contexts and 100
interrupted reading sessions. It also records all 82 Meson unit targets passing.
These are retained results, not new runs from this documentation review.

The native Windows/WSL trial covers voice discovery, two Say All contexts and
orderly process exit. Abrupt Windows cleanup across WSL, self-detaching helpers,
broader browser/terminal interaction, audible latency and daily use remain
unqualified. Say All remains context-level; no word-level progress is claimed.
The adapter is absent from Orca main, and upstream inclusion/publication remain
future milestones. This hardening required no Omnivox protocol change.

## Repository placement

Recommend developing the adapter on a feature branch in the Orca repository,
with upstream inclusion as the preferred destination. Keep this delivery plan
and any Omnivox implementation work in Omnivox. A new repository adds little
value for the initial adapter.

| Material | Recommended home | Reason |
| --- | --- | --- |
| `SpeechServer` implementation, Python transport, Orca settings mapping | Orca: initially `src/orca/omnivox.py`, with a small transport module if needed | These depend on Orca's Python API and event loop. |
| Factory registration, installation, translated labels, user help and adapter tests | Orca | Changes can follow Orca's interfaces and release together. |
| Protocol, synthesis, playback or marker fixes; shared wire fixtures | Omnivox | These serve every client and follow the server's existing gates. |
| Cross-repository delivery plan and server acceptance evidence | Omnivox | Keep future work linked from this roadmap and observations in the evidence archive. |
| Independent plugin distribution | Reconsider after the prototype | Useful only if independent releases are needed and a supported installation/discovery contract is established. |

The planning inspection used Orca `b30a57efa` and Omnivox `e7e7ab6`. Orca already
has `speechserver.SpeechServer`, `speechdispatcherfactory.py` and `spiel.py`.
Both preferences discovery in `speech_manager.py` and command-line validation
in `orca_bin.py.in` use `SPEECH_FACTORY_MODULES`. Meson explicitly installs the
backend sources. Although the speech manager can import an external module,
that alone does not integrate it into the normal selector and command line.
Shipping an independent Python package would therefore still need an Orca
integration strategy.

Do not copy the adapter into both repositories or make Omnivox packages overwrite
Orca's installed Python files. Initial development can use a local Orca branch
without waiting for upstream review. If upstream declines the backend, choose
between maintaining that branch and a supported external-extension mechanism
before committing to separate packaging.

## Initial scope and boundaries

Deliver ordinary speech, characters and key echo, immediate stop, voice and
language selection, common voice settings, and Say All with truthful progress
and interruption. Omnivox remains responsible for engines, PCM and playback;
Orca remains responsible for screen-reader presentation and reading navigation.
Use one persistent worker for the initially serialized speech stream.

The WSL trial can use the existing Emacsvox launcher to start an installed
Windows Omnivox payload and its Eloquence helper. This preserves the stdio
protocol and adds no mandatory Emacsvox dependency to the Orca backend. It does
not implement remote TCP transport. Native Windows cleanup must be observed
separately from the adapter's Linux process-group retirement.

Use the existing public [control](../protocols/control.md),
[timeline](../protocols/presentation-timeline.md) and
[legacy](../protocols/legacy.md) protocols, including their current Emacsvox-named
records. Renaming commands is unnecessary. The existing
[Speech Dispatcher proposal](speech-dispatcher.md) covers Omnivox consuming
Speech Dispatcher output; this plan covers Orca consuming Omnivox speech.

Keep Omnivox optional for Orca. Initial launch resolves the installed `omnivox`
executable without a shell and uses its existing configuration. Missing or
incompatible installations must fail backend initialization cleanly so Orca's
existing backend selection/fallback can operate. Runtime loss interrupts active
speech; recovery must never replay an utterance that may already have sounded.

The first release excludes remote transport, an SSIP server or output module,
new synthesis engines, a reusable Python SDK, native-parameter editing, voice
installation UI, multiple speech lanes and support for older Orca APIs.
Orca selects utterance settings; Omnivox retains installation, engine and output
configuration. The adapter must not persist Orca preferences into Omnivox's
configuration file.

Existing decisions constrain process isolation, cancellation, markers and
settings: [ADR 0001](../adr/0001-engine-isolation-and-distribution.md),
[ADR 0003](../adr/0003-progressive-audio-and-markers.md),
[ADR 0004](../adr/0004-workstation-service-and-worker-ownership.md),
[ADR 0006](../adr/0006-voice-selection-and-customization.md) and
[ADR 0009](../adr/0009-local-speech-preferences.md). This plan does not amend
them. A newly required lifecycle, distribution or public-protocol decision must
follow the [documentation workflow](../DOCUMENTATION-GUIDE.md#adr-lifecycle),
including explicit approval before creating an ADR.

## Adapter contract and qualification targets

### Connection and lifetime

- Negotiate capabilities and inventory before admitting speech. Target the
  existing control envelope 1, `presentation_timeline_v3`,
  `playback_marker_events_v2` and `tracked_playback_completion`; reject a server
  missing the required bundle with an actionable diagnostic.
- Keep reads, writes, startup and retirement off Orca's event loop. Deliver
  callbacks through GLib on the main loop. Bound queued requests and input/output
  buffers using the existing record limits; continuously drain stderr.
- Correlate control replies and playback events with a connection epoch and
  request/dispatch identifiers. Discard callbacks from retired connections or
  reading sessions, including callbacks already queued on GLib.
- Give stop priority over unsent speech. Retire superseded local work and send
  `s` without waiting for synthesis or normal playback completion. Handle a
  partial protocol write without interleaving bytes or deadlocking retirement.
- Own the worker and helper process tree. Stop before normal shutdown and verify
  bounded tree/reader cleanup before replacement. Ordinary stdio EOF drains
  pending speech, so closing stdin is insufficient cancellation. Demonstrate
  behavior on abrupt Orca death as well as orderly shutdown; process groups
  alone do not establish parent-death cleanup. Review existing ownership support
  before proposing any new mechanism or depending on a private management API.
  The implemented POSIX supervisor watches a non-inherited lifetime descriptor
  and retires the owned group on EOF/termination; focused tests cover SIGTERM and
  SIGKILL parent death and a helper ignoring SIGTERM. Detached descendants and
  native Windows processes across WSL remain separate acceptance work.
- After recoverable loss, repeat capability, inventory and logical-voice setup
  and reapply Orca settings for future speech. Bound retries and surface persistent
  failure. A deliberate restart may reload Omnivox configuration; do not claim
  ordinary relaunch preserves the old worker's immutable startup snapshot.

### Speech, voices and settings

- Preserve Orca's order across successive `speak()` calls. Use ordered timelines
  initially; interruption follows Orca's explicit stop operations. Do not make
  every fragment replaceable, because one presentation can contain several calls.
- Use structured logical voices and span ACSS for per-utterance styles. Discover
  engines/physical voices from inventory and retain `(engine_id, voice_id)`
  identity independently of translated display labels. Define and round-trip
  the representation in Orca's existing voice settings before adding schema.
  The prototype retains stable physical IDs in family names and registers used
  voices on demand: inventories can exceed the server's 256-definition limit.
  Each registry acknowledgement precedes speech using newly registered voices.
- Map Orca's default, uppercase, hyperlink and system voice styles and language
  queries. Respect exclusions and actual fallback; do not report an unavailable
  saved voice as the voice that spoke. Use existing language-bearing logical
  definitions; broader language routing remains a separate proposal.
- Begin common ACSS mapping with rate `0..100` to `0..1` and pitch, pitch range
  and gain `0..10` to `0..1`, validating neutral settings and limits on real
  engines. This maps controls, not equal acoustic rates across backends. Identify
  unsupported dimensions and document any range difference.
- Translate plain text into speech spans, not Speech Dispatcher's SSML. Audit
  Orca's existing pronunciation and character-name processing to avoid applying
  it twice and to preserve source offsets and localized key/character names.
- Verify characters, punctuation names, uppercase cues and key echo separately
  from prose. Legacy `l` has special latency/capital behavior but uses the global
  engine route; it cannot be assumed equivalent to a styled logical-voice span.
  Select a path that preserves the requested voice and avoids duplicate cues.
- Map `none`, `some` and `all` directly. The prototype's initial approximation maps
  Orca's `most` to `some`, document the difference and retain Orca's saved choice.
  Exact four-level parity depends on the separately scoped
  [punctuation work](punctuation-configuration.md#stage-two-custom-named-profiles-deferred).

### Say All and source positions

Keep one Orca context in flight initially, requesting the next only after a
successful tracked terminal. Map start/progress, natural completion and
cancellation into `SayAllContext` callbacks. A failure or connection loss
interrupts reading and reports the cause; it must not appear as completion or
advance to the next context. Handle empty contexts without waiting for an event
that cannot occur.

Retain the original context's character offsets. Convert anchor positions to
UTF-8 byte boundaries for Omnivox and correlate semantic-event IDs back to Orca
offsets. Raw engine markers describe processed chunks and cannot be directly
added to the original document offset. Test Unicode, punctuation expansion,
CamelCase, chunk boundaries and nonzero context start offsets.

Use requested semantic anchors only after qualifying the actual engine path.
Respect reported exact, word-boundary, span-boundary or omitted resolution;
never generate progress with elapsed-time estimates. For unsupported paths,
select context-boundary progress before submission and document the resulting
coarser interruption position. Partial playback followed by failure must not
trigger automatic retry using a coarser mode.

The retained [dense-marker failure](../benchmarks/2026-09-28-host-chunk-configuration.md#pre-existing-dense-marker-failure)
reports 14 of 15 requested eSpeak anchors after partial output. Reproduce it
early and test representative Orca anchors. Reliable word-level progress remains
conditional on that investigation; any server fix receives its own regression
coverage and evidence. Do not silently alter the host's chunk-size default.

## Delivery slices and estimates

Estimates are cumulative engineering days for one developer familiar with the
repositories, on the inspected Orca checkout and Linux. They exclude upstream
review delays, older-Orca compatibility, exact `most` punctuation support and
any substantial new server architecture. These are original planning estimates,
not remaining-work estimates or delivery dates. Re-estimate after broader
desktop acceptance.

| Slice | Cumulative estimate | Deliverable and exit criterion |
| --- | --- | --- |
| 1. Integration prototype | 1–2 days | Register/install the optional Orca backend; start a worker, negotiate, select one voice, speak, stop and complete one short Say All sequence. Demonstrate event-loop responsiveness; probe dense anchors and cleanup. |
| 2. Usable adapter | 5–10 days | Preferences and stable voices, common styles, language lookup, character/key handling, continuous reading with qualified or documented coarse progress, bounded interruption and restart. Automated transport/mapping tests and a live desktop smoke test pass. |
| 3. Daily-use qualification | 15–20 days | Fault and race coverage, settings persistence, engine variation, sustained reading/navigation, acoustic checks, installation/help and a reviewable upstream patch series. Remaining limitations and supported versions are explicit. |

Slice 1 should produce a short gap list separating adapter bugs, existing server
bugs and any proposed contract changes. Reassess source placement with that
evidence before creating an independent package. Do not promise full word-level
progress or crash cleanup merely because the prototype speaks successfully.

The integration prototype and part of usable-adapter delivery are implemented.
POSIX parent-death hardening and synthetic sustained-reading checks also pass;
they do not close the daily-use milestone. Next work is broader live desktop and
listening acceptance, abrupt Windows/WSL cleanup qualification and investigation
of finer source progress. Reassess the remaining scope using those results.

## Verification and release readiness

Use Orca's existing unit-test infrastructure plus a controllable fake Omnivox
process for split/oversized/malformed records, delayed replies, blocked pipes,
startup failure, cancellation, late callbacks and worker exit. Exercise multiple
queued `speak()` fragments, terminal-before-GLib-callback races, restart epochs,
settings changes and failed voice selection. Test source mapping with non-ASCII
text and transformations, not only ASCII or whole-context completion.

Live acceptance uses a staged Omnivox payload and the test Orca build in a
separate session; preserve the user's active screen reader. Cover eSpeak and at
least one available helper engine, missing runtime, rapid typing/navigation,
long Say All, stop/restart, engine failure, idle/resume and Orca exit. Demonstrate
no stale speech or reading advancement, no duplicate replay and no surviving
owned helpers after retirement. Include browser and terminal reading, voice
settings save/reload, and switching back to an existing backend.

Record source revisions, build inputs, configuration, engine/voice IDs, commands,
raw results and unavailable coverage. Compare latency against the existing Orca
backend under matched conditions where practical. Keep source-consumption
timing distinct from audible onset/stop-to-silence; choose performance thresholds
from the baseline rather than claiming an unmeasured target.

Run Orca's applicable lint, unit, build/install and integration checks. If server
code changes, follow Omnivox's pinned-toolchain formatting baseline, locked
workspace tests, relevant locked Clippy and runnable-payload gates. Public
protocol/helper changes also retain ADR 0006's full Windows development staging
requirement. Documentation changes run `make docs-check` and `git diff --check`.

Before release, document the qualified Orca/Omnivox versions, required
capabilities, installation, executable discovery, settings precedence,
punctuation/progress limitations, diagnostics and how to select another backend.
Keep the adapter an optional Orca component and Omnivox a separate runtime;
upstream merge and distribution availability remain separate milestones.
