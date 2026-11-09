# Private engine startup snapshot

The [snapshot codec](../../omnivox-tts/src/engine_configuration/snapshot.rs)
implements the frozen launch record required by
[ADR 0008](../adr/0008-extensible-engine-registration.md). Standalone startup and
local owned workers consume this record and expose a read-only acknowledgement.
The local service prepares one shared record for both lanes. Emacsvox verifies
each worker's acknowledgement, and the remote host retains one record per session.
This record is private local startup data; no remote speech
operation accepts it.

## Complete record

The UTF-8 JSON document is bounded to 16 MiB, including the captured native
environment and exact managed-generation JSON. This transport bound does not
increase the public configuration, manifest, argument, timeout or managed-library
input limits. The parser rejects decoded duplicate keys, unknown fields,
trailing content and nesting deeper than 16 containers. A single UTF-8 BOM is
accepted. Every field below is required, including nullable fields. Public
configuration permits null only as an explicit punctuation character value.

| Field | Value |
| --- | --- |
| `schema` | Integer `7` for newly prepared records; historical `1`–`6` remain readable. |
| `speech` | Required in schemas 2–7: object containing required integer `max_chunk_words` from 1 through 100. Unknown fields and null fail. Absent in schema 1, which means the historical fixed 15 words. |
| `speech_defaults` | Required non-null object in schemas 3–7, absent in schemas 1 and 2. Every [saved-default member](engine-configuration.md#saved-speech-defaults) is required, with the same validated ranges. `voice` is required but nullable: null retains the historical default selection. |
| `capital_pitch` | Required non-null object in schemas 4–7, absent in earlier schemas. Both `default` and `engines` are required, with the [public value and registered-ID rules](engine-configuration.md#capital-letter-pitch). Earlier schemas imply the historical fixed cue. |
| `audio` | Required non-null object in schemas 5–7, absent in earlier schemas. All three [audio settings](engine-configuration.md#audio-output-settings) are required with their public types and bounds. Earlier schemas imply built-in file defaults. |
| `punctuation` | Required non-null object in schemas 6 and 7, absent in earlier schemas. All three resolved tables (`none`, `some`, `all`) are required with the [public key/name and size bounds](engine-configuration.md#punctuation-tables). Character values may be null; absent characters preserve their input scalar without inheriting current defaults. Earlier schemas use the historical ASCII table. Schema 7 additionally allows `profiles`, a bounded map of fully resolved `{fallback, table}` objects; omitted means empty. Schema 6 rejects nonempty profiles. |
| `platform` | Native Rust OS name, exactly matching the recipient. |
| `activation_id` | Canonical lowercase UUID, freshly generated for a prepared activation and retained by clones/recovery. |
| `registrations` | Complete registration records, including all reserved shipped IDs and at most 32 external IDs. Duplicate IDs fail. |
| `routing` | All four local policy fields: nullable `preferred_engine_ids` and `fallback_engine_ids`, plus `disabled_engine_ids` and `automatic_engine_ids` arrays. Null preserves omission; an empty array preserves an explicitly empty list. |
| `environment` | Array of native name/value pairs, captured once and shared by helper launch records. Duplicate native keys, empty names, equals signs in names and NULs fail. Windows key comparison follows `std::process::Command`. |
| `root` | Nullable absolute native configuration-root path, retained as provenance without rereading it. |
| `managed` | Nullable managed-provider record described below. |
| `requested` | Resolved CLI/environment startup preference, including existing compatibility aliases. |
| `piper_selected` | Boolean retaining whether Piper participates in startup inventory. |

Native strings use Serde's `OsString` representation: an object with `Unix`
containing a byte array, or `Windows` containing a UTF-16 code-unit array.
Paths use this representation too. Non-Unicode legacy arguments and environment
values survive without lossy conversion; external-manifest inputs retain their
UTF-8 requirements. No shell interpretation occurs.

Historical schemas 1–6 retain their original schema and field shape when
serialized again. Schema 1 cannot carry `speech`; schemas 1 and 2 cannot
carry `speech_defaults`. Both imply the old built-in speech defaults. This permits
retained owners to restart the older executable whose identity they pin. New
preparations always emit schema 7, including all resolved defaults when public
configuration omits settings. The two workers and every recovery attempt consume
the same resolved values without consulting changed files. This private schema increment
does not change the public `engine_configuration_version: 1` acknowledgement.

On Windows, initial environment capture omits inherited drive-directory entries
whose names start with `=`, such as `=C:`. `vars_os()` exposes these hidden
process bookkeeping entries, but they are not launch settings. Restoring a
complete record still rejects equals signs in names; it does not silently repair
an invalid record. All ordinary native names and values remain captured.

Each registration contains required `engine_id`, `origin`, nullable `source`,
nullable `override_source`, `enabled`, nullable `unavailable` and nullable
`helper`. Origins are `in_process`, `shipped_helper` and `external_helper`.
Reserved IDs must match their compiled origin; external IDs retain the manifest
grammar and require an absolute source and helper definition. An in-process
registration cannot contain helper launch data. A disabled registration must
remain unavailable; a local routing exclusion cannot be restored to enabled.

A helper contains required native `program`, native `arguments` array and integer
`startup_ms`, `request_ms`, `synthesis_idle_ms`. The original timeout ranges apply.
External programs and argument vectors retain the public path and size rules.
Legacy shipped-program and argument semantics are preserved. Every helper uses
the record's environment; preparation rejects a divergent helper environment.

A managed record contains required native absolute `path`, UTF-8 `generation`
string holding the exact original JSON bytes, and boolean `piper_override` and
`flite_override`. The managed-library parser validates the retained generation
again. Managed helper argument vectors must still name that generation path and
its exact SHA-256. Parsing does not open the generation or verify assets: native
asset verification and the helper's pinned-generation check remain prerequisites
for loading.

## Ownership and privacy

The UUID identifies retained configuration ownership, not executable bytes,
vendor-library identity or audible output. Parsing creates no native engine,
performs no discovery and reads no mutable configuration or environment. It
cannot turn incomplete launch data into defaults. Input diagnostics are emitted
by the preparing caller; workers consume the retained valid registration set.

Full records contain private paths, arguments and environment values. They have
no debug projection and must not be copied into speech logs or public status.
Public diagnostics may expose activation identity, engine ID, origin,
configuration source and availability. The [acceptance audit](../benchmarks/2026-09-28-engine-framework-audit.md#acceptance-checklist)
records shared-worker, recovery and rollback coverage.
The [control acknowledgement](../protocols/control.md#engine-configuration-acknowledgement)
is projected from the record bound to the worker before engine publication.

## Local owner handoff

The local owner resolves all engine inputs before constructing its child and
saves the complete `engines` record inside its existing retained `Startup`
envelope. Candidate-generation preparation does the same after applying the
candidate's managed-provider inputs. A deliberate new capture generates a fresh
activation UUID. Reusing a retained startup preserves it and does not read the
main configuration or manifests again. Native asset checks still run.

After assigning the child to its owned process tree, the owner writes the exact
seven bytes `START1\n`, then a four-byte unsigned big-endian payload length, then
that many bytes of snapshot JSON. The payload must be nonempty and at most
16 MiB. The worker consumes and validates this whole frame before parsing CLI
startup settings or constructing engines. Following bytes remain ordinary
speech input. Older `START\n` gates are not accepted for a local owned worker.
The remote broker also uses this complete frame after establishing native worker
ownership; its authenticated session supplies the host-prepared snapshot.

Transmission has a ten-second deadline. The writer is retained alongside the
worker's pipe readers; an incomplete send enters the existing owned-tree cleanup
path. A retirement receipt requires the child, descendants and pipe tasks to
finish. A failed transmission does not detach an untracked writer or authorize
replacement of an unretired child.

The retained owner envelope still pins executable identity, argv, working
directory, native environment and optional voice-library configuration. Its
`environment` now serializes native name/value pairs; historical UTF-8 maps
remain readable for inspection and package retention. On Windows only, reading
a historical map omits its inherited drive-directory bookkeeping entries before
validating the remaining settings. New native-pair records retain strict name
validation. The owner envelope's total bound is
33 MiB, allowing both owner/helper environments and existing envelope metadata;
the nested engine record and transmitted payload retain the 16 MiB bound.
Historical envelopes without `engines` are retained conservatively for cleanup,
but cannot restart a current owned worker by silently rereading configuration.
They require a fresh activation. Existing snapshot hashes, private-file creation
and retirement/preparation receipts remain in force.

The process acceptance command is
`python3 tools/verify_engine_configuration.py target/debug/omnivox` after
`make dev`. It exercises two actual Unix owners and helper processes, mutates
files/environment between launches, checks independent retirement and verifies
that a fresh activation rejects the now-invalid main configuration. It also
records actual synthesis requests: both lanes, client reset and an owner
restarted from its retained record must keep the configured three-word limit.
The fixture has two voices and records rate/pitch as well, checking saved
defaults, subsequent client changes and reset even after configuration becomes
invalid. Exact diagnostic CLI/positional overrides and private preview selection
remain independently checked. Configured punctuation is checked through actual
helper requests at all three levels, including explicit preservation, preview,
reset, the second lane and retained owner recovery after files change.
This is
null-output framework coverage, not native adapter or audible qualification.

## Shared preparation through the existing local service

The local `host` reply advertises `engine_configuration_version: 1`. A local
service request `engine-snapshot` prepares a fresh activation before constructing
any speech worker. Its `prepared_startup` reply contains `startup`,
`startup_sha256`, nullable managed `configuration` and `activation_id`. Full
launch data stays in the private retained file.

Both owners can receive that same reference through
`OMNIVOX_OWNED_ENGINE_STARTUP` and `OMNIVOX_OWNED_ENGINE_STARTUP_SHA256`. These
settings conflict with `OMNIVOX_OWNED_STARTUP` or `OMNIVOX_OWNED_LIBRARY`; ambiguous
selection fails. Shared preparation preserves all engine inputs and overlays
only the existing per-lane process audio settings: `ALSA_DEFAULT`,
`SWIFTMAC_AUDIO_TARGET`, `SHARPWIN_AUDIO_TARGET`, `PULSE_SINK` and
`OMNIVOX_AUDIO_TARGET`. Helpers keep the common frozen environment. Each owner
saves its resulting complete startup separately for its own retirement/rollback.

The managed `snapshot` request can additionally carry `startup` and
`startup_sha256` from the first lane's prepared candidate. It reuses that engine
record, checks the managed candidate identity and captures the other lane's
existing audio settings. Omitting the reference prepares a fresh candidate.
The `snapshot` reply includes `activation_id`; `owner` replies include nullable
`activation_id` (null if preparation failed). These owner receipts describe
preparation; clients still obtain the worker's independent control acknowledgement.

Emacsvox's bundled local launcher integration prepares one record for an ordinary
deliberate speech start and compares each actual worker's acknowledgement before
publication. This check precedes ordinary capability/routing readiness hooks.
One-lane recovery reuses the acknowledged activation, including after lifecycle
cleanup clears the process globals. A failed notifier during a fresh activation
degrades to main speech and retires the old activation's notifier. Explicit
managed Apply retains its existing preflight, two-lane replacement and rollback
controller; its candidate shares one record and each rollback lane acknowledges
its own previous record. Older hosts without the preparation capability retain
the previous client startup path.

The Emacsvox opt-in ERT test `omnivox-library-engine-native-pair-freezes-and-recovers`
uses `EMACSVOX_ENGINE_FRAMEWORK_TEST_SERVER` to select a `make dev` Unix payload.
It also accepts a fully staged Windows executable from WSL. For a Windows
payload whose eSpeak data is stored separately, set
`EMACSVOX_ENGINE_FRAMEWORK_TEST_ESPEAK_DATA` to the exact native path recorded in
that runtime's `espeak-ng-data.path`. The fixture keeps host storage in native
Windows temporary storage and launcher log pipes on the WSL filesystem.
It runs in fresh batch Emacs with isolated native storage and null audio, changes
configuration between the lane starts, checks both worker acknowledgements,
recovers main without replacing notifications, and verifies that a deliberate
fresh start rejects the changed invalid configuration.

## Explicit minimal emergency startup

`--emergency-version` reports `OMNIVOX-EMERGENCY 1` without opening a host or
reading preferences. `--emergency-owner` starts one owned eSpeak worker with
built-in host defaults. Its optional `--audio-output device|null|pulse` defaults
to `device`; `pulse` requires Linux and `null` is for diagnostics. Optional
`--espeak-data ABSOLUTE_NATIVE_PARENT` selects the matching installed data bundle
when packaging stores it separately, as in Emacsvox's Windows runtime. Otherwise
data must be beside the executable. Each option occurs at most once. Missing data
fails; the engine does not fall through to environment, build or system data.

Optional `--prepare` constructs and verifies the minimal startup without creating
a worker, initializing an engine or opening output. Its `owner` reply advertises
`deferred_start_version: 1`, `prepared: true` and `retired: false`. The selected
bundled data must be present. This permits preparation while old speech remains
available; it does not promise that subsequent engine/output initialization will
succeed. The client must still verify old native retirement before release.

The identity-bound `start-emergency` owner request consumes the prepared attempt
before native creation. A successful `started` reply names the same `worker`;
it acknowledges launch, not usable speech. Engine registration and client
initialization remain necessary. Retrying a lost reply observes the same attempt;
it never launches another worker. Retirement cancels an unreleased preparation,
and a failed or retired attempt cannot be released again. Executable/startup
verification is repeated immediately before native launch. Failed preparation
can be discarded without native cleanup because it has created no native child;
after any possible release, actual cleanup proof is mandatory.

The owner constructs a bounded in-memory snapshot with only eSpeak eligible and
no helper definitions or managed assets. It strips Omnivox and legacy speech
preference overrides from the child environment, retaining host I/O variables
such as `SystemRoot`, `PULSE_SERVER` and `XDG_RUNTIME_DIR`. Native tree ownership
precedes the existing START frame. It opens no management host, saves no startup
journal and neither reads nor changes current configuration, active pointers,
transaction history or old asset claims. Runtime payload/data must remain usable.

The ordinary `owner` reply now includes `native_retirement_version: 1` and
`emergency: false`. A minimal owner reports `emergency: true`, empty `startup`
and `startup_sha256`, null managed `configuration`, and its actual in-memory
`activation_id`. The worker independently acknowledges that identity. Empty
startup fields are not retained-startup or managed-Apply references. Minimal
workers advertise no host preference reload capability. Ordinary per-request
coherence, queues, cancellation and output behavior remain unchanged.

An explicitly negotiated local owner request `retire-native`, carrying its
exact `worker` UUID and positive `request_id`, attempts native cleanup first and
returns `native_retired` only after the tree and pipe tasks finish. Its fields
are `worker` and Boolean `retirement_recorded`. The latter is true only if an
ordinary owner's existing snapshot has a successfully written durable receipt.
It is false for a minimal owner or failed management write. This response proves
native retirement to the live client; false never releases persisted claims or
substitutes for a retained receipt. The owner stays connected, so retry with a
new request ID observes the same original authority. A spontaneously exited
worker whose tree/pipe cleanup succeeds but receipt write fails similarly reports
an unsolicited `native_retired` with request ID zero and keeps its owner alive.

Before replacing known old speech, clients must obtain that live proof or existing
valid durable cleanup evidence. A disappeared owner without evidence remains
blocked. No saved PID, missing record, EOF or process exit grants new authority.
Emergency startup creates a private instance; it cannot discover or retire unknown
workers from other clients. The current ordinary owner still shares its management
transport with speech forwarding. A blocked owner/pipe or combined owner failure
is not qualified emergency recovery; independent native guard reuse and an OS
recovery boundary remain separate work under
[ADR 0018](../adr/0018-minimal-emergency-speech.md).

The paired client exposes `tts-emergency-start` for the foreground lane, preserves
its healthy peer and uses its original launcher context. It prepares the actual
replacement owner before retiring old speech and then reuses that process,
eliminating a separate capability executable invocation. It requires the deferred
capability for this route; an older runtime rejects `--prepare` before native
initialization and working old speech is preserved. Ordinary speech compatibility
and the immediate native emergency entry remain unchanged. The launcher resolves
only executable and matching data references, bypassing optional preference
conversion and disk logging. Managed Apply rejects an emergency participant.
Repeated `tts-recover` retains minimal mode; `tts-restart` deliberately attempts
ordinary saved settings again. A failed initialization retains the attempted
owner until its native cleanup is proven, preventing another retry from treating
an uncertain attempt as an empty first start. A currently active client-managed
replacement must finish or cancel before emergency replacement; damaged persisted
history alone does not impose this live-operation exclusion.
The [evidence report](../benchmarks/2026-11-10-emergency-speech.md)
separates native source timing, client response and unavailable acoustic checks.
The [deferred-start comparison](../benchmarks/2026-11-10-deferred-start.md) measures
the preparation/release increment. Opt-in `OMNIVOX_STARTUP_TIMING=1` emits owner
startup spans on stderr; it does not instrument speech admission. WSL callers
must explicitly pass this diagnostic variable through `WSLENV`.

## Remote workstation ownership

The broker captures native environment, working directory and executable path
locally and prepares one complete engine snapshot per authenticated session.
It hands the same bounded private frame to each owned worker only after process
tree ownership is established. The pipe writer is retained and joined, including
when startup transmission times out and retires the tree. TCP clients never
provide these launch definitions.

The session retains its snapshot while either or both lanes are disconnected.
Another session can replace it only after both previous workers retire. Service
shutdown ends this in-memory retention; no remote management store is added.
The UUID remains a configuration identity and does not attest executable bytes.
`tools/test_remote_service.py` covers actual lane acknowledgements, changed local
files, recovery after total disconnect, and fresh-session validation. Its opt-in
Emacs acceptance also checks independent recovery and deliberate restart.

### Reconcile an existing retirement receipt

The local management service advertises `retirement_receipts_version: 1` in its
`host` reply. A client may then send `command: "retirement-status"` with its
positive `request_id`, the original owner `worker` UUID, native `startup` path
and `startup_sha256`. This command is available only outside a managed activation;
it is not part of the speech control or remote protocol.

The path must be exactly this host's `sessions/WORKER_UUID.json`. The service
opens the existing retention lock without creating a replacement, rejects busy
storage, and verifies the bounded startup bytes against the supplied SHA-256.
It then inspects only the corresponding `.retired` receipt. Version 1, state
`retired` and the exact snapshot digest must match. A `.prepared` record is never
retirement evidence. Missing retirement evidence yields `retired: false`;
missing or changed startup, malformed receipt, changed identity/path, linked files
and missing or busy locking state yield an error. No record is repaired or removed.

The correlated `retirement_status` reply carries `worker`, `startup_sha256` and
the Boolean `retired`. A client must match the worker and digest and require
`true` before acknowledging cleanup. The native owner writes this durable receipt
only after its tree and pipe tasks finish, before its final reply and exit. The
query therefore reconciles lost replies under
[ADR 0007](../adr/0007-managed-voice-lifecycle.md); process exit or an absent lock
is still insufficient evidence.

Inspection neither loads engines nor rehashes executable/assets: it establishes
past native retirement from the exact persisted proof. A subsequent owner still
performs its normal startup and asset verification and publishes its own retention
record before opening the native gate. The query does not depend on current
preference files or the active pointer, but still requires a usable host identity,
retained snapshot, receipt and existing writable retention lock. It is ordinary
reconciliation, not emergency startup through damaged management state.
