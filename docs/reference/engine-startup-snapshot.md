# Private engine startup snapshot

The [snapshot codec](../../omnivox-tts/src/engine_configuration/snapshot.rs)
implements the frozen launch record required by
[ADR 0008](../adr/0008-extensible-engine-registration.md). Worker handoff and paired
activation acknowledgements are still being integrated. This record is private
local startup data; no remote speech operation accepts it.

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
