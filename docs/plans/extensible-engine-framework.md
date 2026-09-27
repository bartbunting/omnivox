# Extensible speech engine framework

Status: Implementation in progress, 2026-09-27. Configuration version 1 below
defines the authorized first implementation slice. Standalone startup and exact
diagnostics now share strict configuration loading, resolved launch definitions
and provider-owned invocations. Routing and previews retain local selection
permissions; recovery retains launch arguments and environment. External startup
uses a separate bounded batch. Shared activation snapshots and acknowledgements
for paired workers remain pending, as does native platform qualification.
The later language-routing section remains a separate design proposal.

The [roadmap entry](../ROADMAP.md#extensible-engine-registration) tracks the
delivery scope. [ADR 0008](../adr/0008-extensible-engine-registration.md)
records the architectural choices and alternatives; this document defines the
detailed configuration contract and implementation acceptance criteria.

## Purpose and scope

Provide one framework for adding speech engines through isolated helper
processes. A helper can be distributed with Omnivox, installed as a separate
companion, or maintained independently. The runtime it uses can be bundled with
the helper or supplied separately by the user, subject to the applicable
component and distribution policies. These are independent choices.

Adding a conforming external helper must not require recompiling Omnivox or
assigning it another engine's identity. A maintained helper should use the same
registration, inventory, routing and lifecycle machinery. Moving a helper
between distribution arrangements should preserve saved engine and voice IDs.

This framework covers registration, configuration, discovery, selection,
synthesis and failure isolation. Engine installation, managed voice acquisition,
licensing/activation services and automatic language detection have separate
contracts. A registration cannot grant itself any of those capabilities.

The existing [helper protocol](../protocols/helper.md) remains the
process boundary. Accepted ADRs continue to govern
[engine isolation](../adr/0001-engine-isolation-and-distribution.md),
[rate calibration](../adr/0002-speech-rate-calibration.md),
[progressive audio](../adr/0003-progressive-audio-and-markers.md),
[remote ownership](../adr/0004-workstation-service-and-worker-ownership.md),
[voice eligibility](../adr/0007-managed-voice-lifecycle.md),
[native parameters](../adr/0006-voice-selection-and-customization.md), and
[startup fallback](../adr/0007-managed-voice-lifecycle.md).

## Components and responsibilities

| Component | Responsibilities |
| --- | --- |
| Registration loader | Validated local launch definitions and configuration provenance. |
| Engine registry | Stable identities, availability, current descriptors and helper lifecycle. |
| Routing policy | Engine eligibility, ordered voice choices, language preferences and fallback. |
| Helper protocol host | Negotiation, bounded messages, synthesis admission, framing and cancellation publication. |
| Engine adapter | Native loading, threading, text conversion, voice discovery, controls, PCM capture and native cleanup. |
| Omnivox audio pipeline | Conversion, effects, mixing, playback and completion. |
| Client | User interaction, saved preferences and language context; coordinated configuration of its speech workers. |

One unrelated runtime occupies one helper process. Each speech worker owns its
helper instances; foreground and notification speech retain independent state
and cancellation. Process isolation contains faults but is not an operating
system sandbox. Registering an executable authorizes it to run as the user.

The framework has three distinct data sources:

- **Registration:** where and how to start an adapter.
- **Descriptor:** what the selected running adapter can actually do.
- **Policy:** when that adapter and its voices may be selected.

Configuration must not invent voices, declare successful runtime validation,
claim streaming or marker support, or override negotiated capabilities.

## Unified registration model

Normalize all engine sources into one internal registration model:

1. Compiled in-process adapters supply a factory and retain their existing
   process boundary.
2. Shipped helper definitions supply launch metadata tied to the installed
   distribution or companion.
3. User manifests supply explicit local external-helper definitions.

Server startup, inventory, exact previews, diagnostic voice listing and recovery
must consume the same resolved definitions. Avoid separate engine-name switches
that make an engine work in server mode but disappear from diagnostics.

A shipped definition may resolve a packaged executable relative to its verified
installation root. A user manifest must name a fully absolute executable path
on the speech host. Neither source discovers helpers from the working directory,
unrestricted `PATH`, speech input or network searches.

The first version reads registration at worker startup. It supports
adding engines without rebuilding Omnivox, with restart to activate changes.
Live reload is a later lifecycle extension requiring atomic replacement,
in-flight request retirement, removal semantics and coordination between
workers. Saving a manifest must not itself restart speech.

## Configuration version 1

Version 1 implements registration and the existing engine-routing controls,
plus explicit permission for external engines to join unrestricted matching.
It does not implement new language-routing semantics. In particular,
`default_language`, `language_rules`, and `cross_language_fallback` are not
version-1 fields: reject them rather than silently ignoring a language policy.
A later schema/capability increment will add those fields together with their
behavior. Configuration versions are independent of helper protocol versions.

### Configuration root and activation

Choose exactly one root, with no merging across roots:

| Priority | Source |
| --- | --- |
| 1 | CLI option `--config-dir ABSOLUTE_PATH`. |
| 2 | Nonempty environment variable `OMNIVOX_CONFIG_DIR`. |
| 3 | Native Windows: `%APPDATA%\omnivox`. |
| 3 | macOS: `$HOME/Library/Application Support/Omnivox`. |
| 3 | Other Unix: `$XDG_CONFIG_HOME/omnivox`, or `$HOME/.config/omnivox` when that variable is unset or empty. |

Explicit roots and nonempty platform base-directory variables must be fully
absolute native paths. Reject relative values; do not resolve them against the
working directory. Missing required platform variables are configuration errors.
`--config-dir` names a directory, not an individual file. An explicitly selected
root must exist and be readable; an absent default root means no user settings.
The reader never creates directories or writes files.

The root contains optional `config.json` and optional `helpers.d/`. Read only
direct regular files whose names end in `.json`, in UTF-8 filename order. Do not
recurse or follow symlink/reparse-point manifest entries; diagnose such entries.
Resolve the selected root itself once, so a redirected user configuration root
is supported. Ignore editor autosaves and other non-JSON names.

A missing `config.json` means defaults; a missing `helpers.d` means no external
helpers. Existing but unreadable files/directories are not treated as absent.
Use the machine running Omnivox: a Windows worker launched from WSL reads the
Windows root, and a remote speech client cannot supply this launch configuration.

Read and validate before launching external helpers. Registration changes apply
to a new worker, not an existing worker's recovery or current utterance. Initial
activation uses an explicit coordinated restart; saving files never triggers it.
There is no version-1 reload command or new resident configuration service.

### JSON conventions and bounds

Both file types are UTF-8 JSON objects; accept one initial UTF-8 BOM for Windows
writers. Reject comments, trailing commas, duplicate decoded keys, unknown keys
at every level, unsupported schemas and null in place of optional fields.
Integer fields must deserialize as unsigned integers, not floats or strings.
String limits below count UTF-8 bytes after decoding. File limits include a BOM.

| Input | Limit |
| --- | --- |
| `config.json` | 128 KiB. |
| One helper manifest | 64 KiB. |
| Candidate `.json` manifests | 32, including invalid/disabled candidates. |
| Aggregate manifest bytes | 1 MiB. |
| JSON nesting | 16 levels. |
| Engine ID | 1–128 bytes. |
| Program path | 1–4096 bytes, no NUL or control characters. |
| Arguments | 64 strings, each at most 4096 bytes; at most 16 KiB combined. |
| Argument contents | No NUL; other characters remain literal. |
| Each routing list / override object | At most 64 engine IDs / entries. |

Exceeding an individual manifest limit invalidates that registration. Exceeding
the candidate-count or aggregate-size limit rejects the entire external set;
never select an arbitrary prefix. Shipped engines can still supply ordinary
speech, subject to the valid local policy. An unreadable helper directory also
rejects the external set with a diagnostic. Root/main-policy errors use the
stricter behavior below.

### Helper manifest

Every manifest is a complete external registration with these fields only:

| Field | Type | Required / default |
| --- | --- | --- |
| `schema` | Integer | Required; exactly `1`. |
| `engine_id` | String | Required; stable canonical ID. |
| `enabled` | Boolean | Optional; `true`. |
| `program` | String | Required; fully absolute native executable path. |
| `arguments` | Array of strings | Optional; empty array. |
| `timeouts` | Object | Optional; field defaults below. |

```json
{
  "schema": 1,
  "engine_id": "org.example.speech",
  "enabled": true,
  "program": "/opt/example/bin/speech-helper",
  "arguments": ["--runtime", "/opt/example/runtime"],
  "timeouts": {
    "startup_ms": 10000,
    "request_ms": 10000,
    "synthesis_idle_ms": 60000
  }
}
```

A Windows manifest uses an escaped native path such as
`"C:\\Speech\\example-helper.exe"`. Accept drive-absolute and UNC filesystem
paths; reject drive-relative (`C:helper.exe`), root-relative (`\helper.exe`),
device-namespace paths and relative paths. Unix requires a leading `/`. No
shell, tilde, environment-variable or glob expansion is performed. `arguments`
is passed directly as an argument vector, including empty strings and spaces.
The helper validates its own runtime arguments. Program absence is engine
unavailability, not permission to find a different executable.

Version 1 inherits the worker's captured environment and preserves existing
runtime environment overrides. It adds no per-helper environment overlay,
working-directory field, inline script, executable download or descriptor cache
field. Adapters must not depend on the caller's working directory for discovery.

| `timeouts` member | External default | Inclusive allowed range |
| --- | --- | --- |
| `startup_ms` | 10000 | 100–120000 ms. |
| `request_ms` | 10000 | 100–30000 ms. |
| `synthesis_idle_ms` | 60000 | 100–300000 ms. |

Omitted members independently use defaults. Reject out-of-range values instead
of silently clamping. Shipped helpers retain their current qualified defaults;
these ranges constrain new user overrides, not historical protocol behavior.
The ordinary request timeout does not relax the shorter native-metadata query
deadline. Cancellation grace and cleanup limits are not configurable.

At most four external helper initializations run concurrently. Prioritize an
explicit startup engine, then external IDs lexically. A 120-second batch
admission deadline starts when external initialization begins; each operation
also retains its own deadline. At the batch deadline retire outstanding attempts
and mark unstarted helpers unavailable with a startup-budget reason. Cleanup
retains its separate bounded lifecycle; unconfirmed cleanup forbids replacement.
Initialize shipped engines independently so an external queue cannot consume
all their initialization slots. Do not retry external initialization in a loop
during startup. Later explicit rescans/recovery use the existing admission rules.

### Main configuration

`config.json` permits only `schema`, `routing` and `engine_overrides`. `schema`
is required and exactly `1`; the other objects default to empty objects. The
minimum document is `{"schema":1}` and preserves existing behavior.

```json
{
  "schema": 1,
  "routing": {
    "preferred_engine_ids": ["espeak"],
    "fallback_engine_ids": ["espeak"],
    "disabled_engine_ids": [],
    "automatic_engine_ids": []
  },
  "engine_overrides": {
    "org.example.speech": {
      "timeouts": {"synthesis_idle_ms": 90000}
    }
  }
}
```

Each routing member is an array of canonical ID strings. Reject duplicates
within a list; the same ID may appear in different lists, as in the existing
policy API. Preserve list order. Unknown but valid routing IDs remain inert
references with an unavailable diagnostic, so policy can survive an engine's
removal or installation on another machine. They never cause executable search.

| `routing` member | Omission | Explicit empty array |
| --- | --- | --- |
| `preferred_engine_ids` | Existing platform startup preference. | No configured preference; use ordinary shipped startup defaults. |
| `fallback_engine_ids` | Existing server fallback behavior. | No policy engine fallback; explicit logical alternatives still apply. |
| `disabled_engine_ids` | No additional local disablement. | No additional local disablement. |
| `automatic_engine_ids` | No external engine joins unrestricted matching. | Same as omission. |

Startup still needs a usable default engine when the configured preferred list
is empty. Choosing that default does not populate an explicitly empty policy
list. Fallback lists govern voice resolution after startup; initial engine
creation retains the startup selection sequence specified below.

`automatic_engine_ids` is an additional permission set for enabled external
registrations. It does not reorder engines, select the startup engine, enable a
disabled engine, or remove existing shipped-engine eligibility. Other external
engines remain selectable only through explicit selectors or policy references.
Local disablement is a floor: session policy can add disabled engines but cannot
remove local disablement. Voice-library exclusions remain authoritative too.

`engine_overrides` maps existing registry IDs to objects containing only
`enabled`, `program`, `arguments`, and `timeouts`. Omitted members preserve the
resolved registration value. `arguments` replaces the entire argument vector;
`timeouts` merges by member. Unknown override IDs are main-configuration errors,
not new registrations. An in-process adapter accepts only `enabled`; reject
helper launch fields for it. An override cannot change identity, origin,
capabilities or automatic-selection permission.

Provider-owned managed load arguments cannot be replaced by local `arguments`.
Reject that combination when managed configuration controls the invocation; preserve
existing explicit legacy model/data override semantics and all provider checks.
A missing or conflicting external registration cannot be resurrected through an
override. An override targeting such a registration is a main-configuration
error, including when the external set has been rejected. Optional-manifest
failure isolation therefore applies when the main configuration remains valid;
explicit launch overrides require their referenced registration. Runtime-file
selection remains the adapter's responsibility.

### Identity, conflicts and error behavior

New external IDs match `[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*` within the byte limit.
Comparison is exact and case-sensitive. Dotted publisher namespaces are
recommended but optional; no reserved `local.` prefix forces a later rename.
Existing canonical IDs retain their spelling. Configuration routing/override
references accept the existing ID syntax; aliases such as `native` are accepted
only by the existing CLI/environment compatibility path and normalize there.

Reserve every shipped/in-process engine ID and legacy alias, even when its
platform adapter, feature or companion is unavailable in this build. An
external registration cannot shadow one. Duplicate otherwise-valid external IDs
invalidate all registrations for that ID, including disabled entries; there is
no last-file-wins behavior. A conflicting file cannot invalidate the reserved
shipped engine. A schema-invalid file supplies no registration.

The descriptor ID must equal the registered ID, and all physical voices must
belong to it. Preserve engine and voice IDs when an independently maintained
helper becomes shipped. If its formerly external manifest is still present,
report the collision and use the shipped definition; launch overrides must then
be explicit through the normal override mechanism, never imported silently.

| Failure | Required result |
| --- | --- |
| Bad selected root, unreadable/malformed `config.json`, unsupported main schema, invalid override | Reject startup before native engine construction; no silently permissive defaults. |
| Invalid individual manifest / duplicate external ID | Diagnose file and reason; affected external registration is unavailable. |
| External-set bounds exceeded / unreadable helper directory | Reject external set; retain valid policy and shipped registrations. |
| Disabled registration | Do not construct, spawn or recover it; retain identity/reason. |
| Missing executable / runtime, descriptor mismatch, handshake failure | Mark engine unavailable through existing lifecycle; ordinary speech may use eligible fallback. |
| Exact diagnostic or preview targets unavailable engine | Report that target's failure; do not substitute another voice. |
| No eligible engine can supply ordinary startup speech | Fail startup with a bounded diagnostic. |

An explicit startup `--engine` remains a preference for ordinary server mode,
with its existing fallback behavior; it is not an exact-preview instruction.
Configuration errors must identify a source and field without echoing argument
contents, private runtime data or speech text. Existing processes are never
reconfigured on a failed new startup. The client/owner retains or restores its
previous valid worker configuration under the existing activation contract.

## Configuration and compatibility

Move deployment choices into validated data while keeping native behavior and
safety invariants in code. Use shipped defaults so ordinary installations do
not require configuration files.

| Current responsibility | Proposed home |
| --- | --- |
| Helper IDs, platform filenames and adjacent companion layouts | Shipped registration metadata plus external manifests. |
| Legacy helper environment-variable names and aliases | Compatibility metadata feeding the same registration loader. |
| Helper arguments and local runtime selection | Local launch configuration, interpreted by the adapter. |
| Startup/request/synthesis-idle timeout defaults | Registration defaults and bounded local overrides. |
| Startup engine preference | Configurable ordered policy with existing platform defaults. |
| Session preferred/fallback/disabled engines | Existing versioned routing policy, extended rather than duplicated. |
| Per-language voice/engine preferences and cross-language fallback | Proposed versioned routing policy. |
| Voices, languages, capabilities and native parameter catalogue | Live helper descriptors and negotiated metadata. |
| Native entry points, ABI, thread ownership, text conversion and calibrated mappings | Adapter code and qualified adapter data. |
| Framing, validation, hard limits, cancellation watchdog and PCM commitment | Common host/server code. |
| Built-in OS adapter construction and compiled feature availability | Compiled factories. |
| Managed asset verification, download/removal operations and release provenance | Existing provider and packaging implementations. |

The current duplication is concentrated in
[`omnivox-cli/src/engine.rs`](../../omnivox-cli/src/engine.rs): helper lists,
filename/environment mappings, startup order, timeout exceptions and separate
exact-engine creation. There are also specialized inventory-cache/prewarm paths
and [managed-provider hooks](../../omnivox-cli/src/voice_library.rs). Extract
static launch data first. Cache trust, provider asset validation and native
initialization behavior need explicit interfaces; arbitrary manifest flags must
not bypass them.

### Resolution order and immutable snapshots

Resolve launch fields in this order, highest priority first:

1. Existing explicit CLI launch/runtime options, for the fields they already own.
2. Existing engine-specific environment overrides, preserving their current
   empty-value and precedence behavior.
3. `config.json` overrides for that engine.
4. The external manifest or shipped registration definition.
5. Framework defaults for omitted fields.

This is field-specific precedence, not permission to merge arbitrary argument
strings. `--engine` / `OMNIVOX_ENGINE` selects a preferred engine; it does not
rewrite that engine's launch fields. Disabled registration, local routing
exclusions and voice-library exclusions cannot be overridden by preference.
No new generic `OMNIVOX_<ID>_HELPER` name is inferred for external engines.

For startup selection, existing `--engine` takes precedence over nonempty
`OMNIVOX_ENGINE`, then the first available locally preferred engine, then the
existing shipped platform order. External engines join this sequence only when
explicitly named. `automatic_engine_ids` does not append external engines to the
startup fallback list. Apply eligibility before considering any candidate.

After connection, a valid client routing-policy registration replaces the
preferred/fallback lists under the existing generation rules. Effective disabled
IDs are the union of local and session disablement. `automatic_engine_ids` stays
local and cannot be widened by a client; explicitly naming an external engine
in a session rule permits it in that rule's scope, not in unrelated automatic
matching. A client that does not register policy uses the resolved local/default
policy. Keep the distinction between an omitted list and a supplied empty list.

Normalize and freeze the complete launch snapshot before constructing workers.
It contains registrations, resolved paths/arguments/deadlines, enablement, local
policy, source identity, applicable managed-provider inputs and the captured
launch environment. Recovery clones this snapshot rather than rereading files
or a changed environment. It must not change engine identity or bypass renewed
runtime validation after a crash.

Identify each prepared snapshot with one opaque activation UUID shared by the
workers launched from it. The UUID identifies retained configuration, not a
hash, file signature or proof of what is loaded. Local orchestration passes the
same owned snapshot to both workers; a worker must not independently reread
mutable files and merely echo a supplied UUID. The client/owner compares their
actual configuration acknowledgements before completing coordinated activation.
An ordinary standalone invocation owns its own snapshot and UUID.

The private snapshot handoff belongs to the existing local worker/owner path.
Its transport must carry the complete frozen configuration, preserve the input
bounds above and reject partial snapshots before engine construction. It is
local startup data, never a remote speech operation accepting executable
definitions. Reuse existing activation ownership, retirement and rollback.
Recovery preserves the activation UUID; a deliberate new activation creates a
new one even when the files have identical content. Snapshot identity does not
attest binary bytes, loaded vendor libraries, or audible output.

Keep the normalized launch record and environment private. Diagnostics expose
the activation UUID, engine ID, origin/configuration source and availability;
raw environment and argument values are not included. Reading config or probing
a helper never writes files, installs libraries or publishes releases.

## Selection eligibility

Discovery, permission to synthesize and permission to participate in automatic
matching are separate. External helpers default to explicit selection. Their
registration authorizes launch and discovery, but not a change in the user's
default speech merely because a matching voice appeared.

| Selection source | External helper eligible by default? |
| --- | --- |
| Exact physical voice or explicit engine selector | Yes, if enabled and available. |
| Preferred/fallback policy explicitly naming the engine | Yes, within that policy's scope. |
| Future language-specific policy explicitly naming its voice or engine | Yes, for that language rule when implemented. |
| Unrestricted language/gender matching across inventory | No, unless named in local `automatic_engine_ids`. |
| Administratively disabled engine or voice | No, regardless of selector or preference. |

A preference-list position alone does not enforce these rules. Apply eligibility
at each resolution stage and again before synthesis, including previews,
default selection, language matching and recovery. Preserve the existing
eligibility behavior of shipped engines during the registration migration.

## Language selection

### Existing behavior

A [voice descriptor](../../omnivox-tts/src/contracts.rs) has one optional
language string. A selector can name an exact voice, an engine default, or
properties including language and gender. A logical voice has a separate
optional language value. Tag validation currently checks length and allowed
characters; it is not full BCP 47 validation.

The [resolver](../../omnivox-tts/src/resolver.rs) works as follows:

1. Try the logical voice's ordered selectors. Language constrains a property
   selector only when that selector includes it.
2. If enabled, use the logical voice's language to try another voice on the
   engine named by its first selector.
3. Try global preferred engines, the configured global default selector, then
   fallback engines. Preferred/fallback engine entries select engine defaults;
   they do not automatically retain the logical voice's language constraint.

Property matching compares language tags case-insensitively for exact equality.
`en` therefore does not match `en-AU`, and `fr-CA` does not match `fr-FR`.
Matching across engines follows preferred engine order, then engine ID; within
an engine it prefers a matching default voice, then voice ID. It does not rank
regional variants or discover the text's language.

Text-aware routing also checks the engine's character repertoire. This prevents
known encoding loss; it does not establish that a voice pronounces the requested
language. The `language_switching` capability is not an automatic detector or
an implemented general per-span language-routing instruction. Legacy global
language commands are [deprecated](../../omnivox-core/src/command.rs).

Emacsvox already exposes logical-language and property selectors, configurable
engine priority/fallback, and language-grouped voice browsing. Grouping voices
under a language in the UI does not change the server's exact-match semantics.

### Proposed direction

This is a later versioned increment; none of the following adds a version-1
configuration field. Make language an explicit routing input with predictable
fallback, while preserving exact voice choices. Add this through negotiated
routing semantics; do not silently reinterpret existing selectors or change old
saved policies.

- Accept explicit language context for a speech request or span, with the
  logical voice's language and then a configured default as fallbacks. Capture
  this context with the admitted request, not mutable process-global state.
- Add ordered language rules selecting voices or engines, independent of how
  those helpers were installed. Prefer the requested language across eligible
  engines before permitting a change of language.
- Keep explicit physical choices authoritative. Exact previews remain exact.
  Language policy fills automatic choices and permitted fallback; it does not
  silently replace an explicitly chosen voice because metadata differs.
- Separate exact tag matching from language-range matching. Retain legacy exact
  matching; offer [RFC 4647 basic filtering](https://www.rfc-editor.org/rfc/rfc4647.html#section-3.3.1)
  for explicit range rules. A range `fr` can match `fr-CA` or `fr-FR`; `fr-CA`
  does not directly match `fr-FR`. An ordered rule can prefer `fr-CA`, then `fr`.
  Do not infer relationships between distinct languages or script variants.
- Within a language match tier, apply explicit voice/engine preferences, then
  deterministic existing defaults. For automatic routing, try the most specific
  configured language tier across its eligible engines before a broader tier.
- Make cross-language fallback an explicit policy choice: fail the route, or
  use a configured fallback and report language degradation. Unknown language
  metadata cannot satisfy a strict language requirement. Unlabelled requests
  retain ordinary configured default behavior.
- Expose requested language, matched voice language, resolution stage and any
  relaxation in bounded diagnostics. Preserve successful ordinary fallback
  when the policy permits it; do not label a different-language voice an exact
  match.

As a conceptual policy, a French-Canadian request might try a selected `fr-CA`
voice, another eligible `fr-CA` voice, then an explicitly permitted `fr` range.
Only a separate cross-language fallback rule would allow an English default.
The concrete storage and wire schema must make these stages explicit.

Initially, clients can use the existing language-bearing logical definitions
and property selectors. A later negotiated extension is needed for per-request
or per-span language context and changed matching/fallback semantics. Language
rules should extend the existing generation-safe policy, not introduce another
independently mutable routing table. Both speech workers and previews must use
the same admitted semantics.

Automatic detection and mixed-language segmentation remain optional later
features. They need confidence, override and short-text behavior of their own.
A future client detector can supply explicit context without changing the
helper registration contract. Engines needing an in-voice language argument
or multiple advertised languages require a negotiated descriptor/request
extension; selecting a voice is sufficient for the initial single-language
voice model.

## Helper contract

Reuse the existing bounded JSON-lines protocol over stdin/stdout, with stderr
for diagnostics. The framework supports engine-specific helpers in any language;
internal C# or Rust interfaces are implementation aids rather than a new binary
plug-in ABI.

| Adapter operation | Contract |
| --- | --- |
| Initialize | Select and validate a runtime and establish native ownership, or report unavailability. |
| Describe | Report stable engine/voice IDs, actual availability, language, default voice, PCM format and truthful capabilities. |
| Synthesize | Accept the selected voice, text, normalized controls, optional anchors and negotiated native settings. |
| Emit | Return bounded PCM and markers in the announced frame clock and protocol order. |
| Stop | Observe cancellation, coordinate native interruption and suppress stale callbacks/output. |
| Dispose | Release native resources only after use ends; uncertain cleanup prevents reuse. |

Helper 5 is a practical initial target for a new adapter, supporting buffered
or genuine progressive PCM and the common controls. Helper 6 adds the existing
optional [typed native-parameter contract](../protocols/engine-voice-parameters.md).
Preserve all negotiated older-version shapes. Discovery of optional native
controls must not interrupt speech or become necessary for basic synthesis.

A separately installed runtime must not be a required import at process load.
Absent or rejected libraries still allow `hello`, `ping` and `shutdown`, with
`not_available` for runtime-dependent requests. Explicit library paths and
registered runtime identities use documented, restricted loading rules. Validate
architecture, required interfaces and resources; a failed explicit selection
must not silently select another installation. Paths are native to the speech
host, not the client.

The adapter converts wire UTF-8 to its native encoding and must
reject unrepresentable text rather than replace characters silently. Encoding
repertoire, spoken language and voice identity remain distinct. The current
finite repertoire enum may need a negotiated extension for additional encodings;
a user configuration string cannot claim support the host cannot validate.
Native command insertion must preserve original UTF-8 source mappings before
advertising text-range markers.

Return PCM to Omnivox rather than playing directly. Publish streaming support
only when audio is emitted during synthesis. Keep native queues bounded under
backpressure. Markers are optional; timing and source offsets must be truthful,
and progressive markers must precede the audio reaching their frame. The server
handles continuous conversion, effects and playback completion.

Use calibrated or explicitly provisional common-control mappings. Reset native
state between independent requests, including cancellation and failures. The
host handles framing, terminal response ordering and protocol limits; an adapter
must not duplicate that machinery or accept executable commands from speech
configuration.

## Failure and lifecycle behavior

One helper admits one active synthesis. Cancellation acknowledgement and the
target's terminal response are distinct. The protocol must remain responsive
while native synthesis is active, and parent-side retirement remains available
if native work or stop blocks. The existing 250 ms cancellation watchdog is a
retirement trigger, not a guarantee of complete cleanup or physical silence
within that time. Replacement waits for confirmed cleanup.

Reuse engine health, circuit breakers and recovery probes. Missing or failed
optional engines leave eligible fallback speech available; exact operations
report an unavailable target. Preserve administrative exclusions through
recovery. Do not replay cancelled speech or splice another engine into an
utterance after PCM commitment. Output-device failure is separate from engine
failure and must not cause cross-engine replay.

Retain hard framing/audio/marker limits from the
[protocol reference](../protocols/helper.md). Configuration can choose
operational deadlines within bounds; it cannot disable validation, loosen
cancellation ownership or bypass managed asset verification.

## First implementation slice

Deliver and verify these changes in separate commits:

1. Establish the strict configuration reader and shared registration metadata,
   preserving existing engine defaults and compatibility inputs.
2. Resolve immutable launch definitions and connect startup, exact diagnostics
   and bounded helper initialization to the same registry.
3. Enforce local/session exclusions and external automatic-selection permission
   throughout resolution, previews, synthesis and recovery.
4. Carry complete prepared snapshots through local ownership and paired Emacsvox
   activation, including acknowledgements and rollback.
5. Complete the process-based fake-helper acceptance matrix and reconcile current
   references, guides and qualification status with verified behavior.

Tests accompany each implementation commit; the final matrix adds coverage across
the complete path. Native integration qualification remains separate.

Implement the version-1 registration/configuration reader, normalized registry
and shared launch resolution, then connect server startup, exact diagnostics,
selection and recovery. Preserve existing engine behavior without new settings.
External registration and maintained helpers use the same lifecycle. Keep all
current native adapter, provider verification and release behavior intact.

Activation is restart-based. The first slice includes local/session disablement
composition, external automatic-selection permission and coordinated startup
snapshots. It does not include new language matching, per-span language context,
a configuration UI, per-helper environment overlays, new caching/prewarming
policy, executable installation or live reload. Those features cannot appear as
accepted-but-ignored configuration fields.

A redistributable fake helper supplies deterministic descriptors, short PCM,
controlled failures and cancellation barriers. It must be launchable from a
path containing spaces and register a previously unknown engine ID. Exercise
that helper through the actual process client, not just a parser or mocked
registry. A real third-party runtime is not needed to prove registration.

## Acceptance checklist

All items below are implementation acceptance criteria, not claims of tests
already performed for this document.

- [ ] **Compatibility:** no files, an absent default root, and `{"schema":1}`
  preserve platform defaults, aliases, environment overrides, shipped inventory,
  exact diagnostics, provider arguments and existing engine order.
- [ ] **Root resolution:** test CLI/environment/default precedence on Windows,
  macOS and Unix; WSL launches use the native worker's root. Relative/empty
  explicit roots, missing variables and unreadable selected roots fail as
  specified. No lookup uses the working directory or network.
- [ ] **Strict reader:** unknown/duplicate/escaped keys, null, wrong types,
  noninteger/overflow values, unsupported schemas, trailing content and malformed
  UTF-8 fail before spawn. A single UTF-8 BOM works. Test every stated size/count
  bound at and beyond its boundary, including disabled/invalid manifests.
- [ ] **Paths and arguments:** absolute Windows drive/UNC and Unix paths work;
  drive-relative, root-relative and device paths fail. Spaces and empty arguments
  survive exactly; shell-looking text is never expanded by the launcher.
  Symlink/reparse manifest entries and editor autosaves follow the stated rules.
- [ ] **Identity:** shuffled file order produces the same registry; duplicate
  external IDs all fail, reserved IDs remain intact on every platform/feature
  combination, and descriptor/voice ownership mismatches are rejected.
- [ ] **Overrides:** exercise every precedence layer, partial timeout merging,
  complete argument replacement, unknown IDs, in-process restrictions and managed
  argument conflicts. Explicit enablement can override manifest disablement but
  cannot remove routing exclusions, restore an invalid/missing registration or
  relax native asset checks. A rejected external set with a dependent launch
  override fails main-configuration validation.
- [ ] **Timeouts and startup:** test default values and both edges of every
  allowed range. Four-slot external initialization prioritizes the selected
  engine, bounds the batch, retires timed-out children and preserves shipped
  initialization. Blocked reads/writes/native initialization retain hard cleanup
  and cancellation rules; no repeated startup retry loop develops.
- [ ] **Registration path:** add a fake helper by writing one manifest, then
  start a fresh server. Verify its real engine ID and voices in inventory,
  exact voice listing/preview and normal speech, with no server rebuild.
- [ ] **Selection matrix:** exact voice, explicit engine, local/session preferred
  and fallback lists may select an external engine in their defined scope.
  Unrestricted property matching and startup defaults cannot select it merely
  because it was registered. Local automatic opt-in permits property matching
  without changing startup order.
- [ ] **Disablement:** manifest/override `enabled:false` causes no spawn or
  recovery. Local routing and voice exclusions survive session replacement,
  exact requests, preview and recovery. Session disablement remains reversible
  within its own scope; it cannot clear the local floor.
- [ ] **Failure isolation:** malformed optional files, an absent executable or
  runtime, bad descriptors, crashes and hangs preserve unrelated eligible speech.
  Invalid main policy fails startup instead of enabling excluded voices. Exact
  operations never report successful substitution.
- [ ] **Protocol and audio:** negotiate supported helper versions, exercise
  buffered/progressive/empty synthesis and bounded markers/PCM, and reject
  malformed or miscorrelated frames through the existing client. Cancellation
  suppresses late PCM; post-commit failure never replays through another engine.
- [ ] **Recovery snapshot:** change/delete files and change the launching
  environment after startup; recovery uses the retained selection and environment
  and revalidates the actual runtime. A new activation reads the new settings.
  Unconfirmed child cleanup prevents replacement.
- [ ] **Two workers:** foreground and notification use the same prepared snapshot
  and acknowledge it independently. Mutate files between their launches to prove
  neither rereads them. Verify independent cancellation, partial-startup rollback,
  reconnect and fresh activation under the existing owner lifecycle.
- [ ] **Distribution independence:** run the same fake adapter as an external
  registration and as a shipped-definition fixture, with identical physical IDs
  and protocol behavior. Promotion collisions are diagnosed and never import
  an old executable override silently. Package no private runtime to pass this
  test.
- [ ] **Diagnostics:** identify activation, source, engine and failure without
  dumping arguments, environment or speech. Report retained unavailable routing
  references and explicit startup fallback distinctly from successful exact
  selection. Configuration reads and tests leave user files unchanged.

Native adapter acceptance remains a separate gate for an actual maintained
engine: runtime loading, voices, encoding, controls, real PCM, cancellation and
repeat use on each supported platform/ABI. Compile-only, protocol, native PCM
and listening results must remain distinguishable. Changed Windows helpers and
protocols use Emacsvox's full development staging before live acceptance.

The later language increment requires its own negotiated schema and paired
client/server coverage for exact/range matching, region/script tags, unknown
metadata, explicit choices, cross-engine same-language fallback, cross-language
policy, previews and both workers. Its future configuration example must carry
a new schema number; version-1 readers reject it in full.
