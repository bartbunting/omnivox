# Private engine startup snapshot

The [snapshot codec](../../omnivox-tts/src/engine_configuration/snapshot.rs)
implements the frozen launch record required by
[ADR 0008](../adr/0008-extensible-engine-registration.md). Standalone startup and
local owned workers consume this record and expose a read-only acknowledgement.
The local service can prepare one shared record for both lanes. Emacsvox
coordination and remote-host session freezing are still being integrated.
This record is private local startup data; no remote speech
operation accepts it.

## Complete record

The UTF-8 JSON document is bounded to 16 MiB, including the captured native
environment and exact managed-generation JSON. This transport bound does not
increase the public configuration, manifest, argument, timeout or managed-library
input limits. The parser rejects decoded duplicate keys, unknown fields,
trailing content and nesting deeper than 16 containers. A single UTF-8 BOM is
accepted. Every field below is required, including nullable fields. Public
configuration retains its separate no-null rule.

| Field | Value |
| --- | --- |
| `schema` | Integer `1`. |
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
configuration source and availability. The implementation plan tracks the
remaining [paired-worker acceptance](../plans/extensible-engine-framework.md#acceptance-checklist).
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
The remote broker's existing gate remains separate until its integration.

Transmission has a ten-second deadline. The writer is retained alongside the
worker's pipe readers; an incomplete send enters the existing owned-tree cleanup
path. A retirement receipt requires the child, descendants and pipe tasks to
finish. A failed transmission does not detach an untracked writer or authorize
replacement of an unretired child.

The retained owner envelope still pins executable identity, argv, working
directory, native environment and optional voice-library configuration. Its
`environment` now serializes native name/value pairs; historical UTF-8 maps
remain readable for inspection and package retention. Its total bound is
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
that a fresh activation rejects the now-invalid main configuration. This is
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
