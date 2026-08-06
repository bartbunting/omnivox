# Omnivox Control Protocol

Omnivox retains the Emacspeak line protocol for speech presentation and adds a
separate versioned channel for discovery, configuration, and diagnostics. The
control channel uses Base64-encoded UTF-8 JSON so native engine and voice IDs do
not need Tcl escaping and remain separate structured fields.

## Compatibility Boundary

- Existing clients and legacy commands continue to work unchanged.
- `emacsvox_tx` remains the presentation-transaction frame. It is not used for
  persistent logical-voice configuration or discovery.
- A client must successfully request capabilities before using later control
  extensions.
- The server advertises only features it currently implements. Version 1
  currently implements capability negotiation, active-engine inventory, and
  atomic logical-voice registration.

## Request Record

A client writes one ordinary protocol command:

```text
omnivox_control {BASE64_JSON}
```

The decoded JSON envelope contains a protocol version, a client-chosen request
ID, and a tagged request body. The current request is:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "capabilities"
}
```

The Base64 field must not contain line breaks. Omnivox rejects an encoded field
before decoding if it cannot fit within the 256 KiB decoded-payload limit.

The structured inventory request uses the same envelope with `"type":
"inventory"`.

A client replaces its complete logical-voice registry with a registration
request. Clients should begin at generation 1 and increment the generation
whenever the definitions or fallback policy change:

```json
{
  "protocol_version": 1,
  "request_id": 43,
  "type": "register_logical_voices",
  "registry_generation": 1,
  "definitions": [
    {
      "id": "source-code",
      "language": "en-US",
      "preferences": [
        {
          "kind": "exact",
          "engine_id": "winrt",
          "voice_id": "winrt:David"
        },
        {
          "kind": "engine_default",
          "engine_id": "espeak"
        }
      ],
      "acss": { "rate": 0.6, "average_pitch": 0.4 }
    }
  ],
  "fallback_policy": {
    "allow_same_language_on_requested_engine": true,
    "global_default": null,
    "fallback_engines": ["espeak"]
  }
}
```

Omitting `fallback_policy` selects the empty default policy.

## Response Record

The server writes one flushed, newline-terminated event to standard output:

```text
__OMNIVOX_CONTROL__ BASE64_JSON
```

A successful capability response decodes to this shape:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "capabilities",
  "server_version": "1.3.0",
  "supported_protocol_versions": [1],
  "features": [
    "control_v1",
    "engine_inventory",
    "legacy_commands",
    "logical_voice_registration",
    "stable_voice_ids"
  ]
}
```

The request ID lets Emacsvox distinguish simultaneous main and notification
process responses. Server events are never injected into synthesized speech.

An inventory response contains an `inventory_generation` and an `engines`
array. Each engine includes its stable ID, runtime availability and health,
capabilities, discovered physical voices, and default voice ID. The current
single-engine server snapshots this descriptor before the reader loop starts,
so inventory requests cannot block stop commands behind synchronous synthesis.
The future registry will increment the generation whenever its inventory
changes.

A successful registration response reports the inventory generation used and
one resolved or unresolved binding for every definition:

```json
{
  "protocol_version": 1,
  "request_id": 43,
  "type": "logical_voices_registered",
  "inventory_generation": 1,
  "registration": {
    "registry_generation": 1,
    "bindings": [
      {
        "status": "resolved",
        "resolution": {
          "logical_voice_id": "source-code",
          "requested": {
            "kind": "exact",
            "engine_id": "winrt",
            "voice_id": "winrt:David"
          },
          "realized": {
            "engine_id": "winrt",
            "voice_id": "winrt:David"
          },
          "reason": "preferred",
          "failed_attempts": []
        }
      }
    ]
  }
}
```

Registration is an atomic whole-set replacement. A newer generation validates,
normalizes ACSS values, replaces the stored definitions, and resolves them
against the current inventory. An identical retry of the current generation is
idempotent and re-resolves against current availability. Reusing that
generation with different content or sending an older generation is rejected
without changing the registry. A valid but presently unresolvable definition
is retained and returned with `"status": "unresolved"` and diagnostic attempts;
it is not silently dropped.

The current speech queue still uses legacy voice state. Registration and
resolution are available for client integration and diagnostics, but per-span
engine routing begins with the engine-registry phase.

## Errors

Errors use the same response envelope:

```json
{
  "protocol_version": 1,
  "request_id": 42,
  "type": "error",
  "code": "unsupported_version",
  "message": "unsupported control protocol version 2; supported version is 1"
}
```

The error codes are:

- `malformed_request`: invalid Base64, UTF-8, JSON, or request shape;
- `unsupported_version`: a well-formed request uses an unsupported version;
- `payload_too_large`: the encoded or decoded size bound was exceeded;
- `invalid_configuration`: a registration violates ID, count, or field bounds;
- `stale_generation`: the registration generation is older than the stored
  generation;
- `generation_conflict`: the stored generation was reused with different
  content.

If decoding fails before the request ID can be trusted, `request_id` is `null`.
Errors do not mutate speech, queue, engine, or logical-voice state. A
registration accepts at most 256 definitions, 32 selectors per definition, and
a 256 KiB decoded JSON payload. Logical and engine IDs are bounded ASCII tokens;
physical voice IDs may contain native punctuation but not control characters.

## Structured Voice Data

The Rust engine, voice, logical-definition, selector, fallback, ACSS, and
resolution types have stable JSON representations for subsequent version 1
messages. Exact physical identities are represented as separate fields:

```json
{
  "kind": "exact",
  "engine_id": "winrt",
  "voice_id": "winrt:HKEY_LOCAL_MACHINE\\SOFTWARE\\...\\David"
}
```

No consumer should construct or parse a combined `engine:voice` string.
Display names are presentation only and must not be used as stable identity.

## Evolution Rules

- New request and response types may be added to version 1 when old clients can
  safely ignore the advertised feature.
- A breaking field or semantic change requires a new protocol version.
- Clients must ignore unknown capability feature strings.
- Omnivox must keep payload bounds, structured terminal errors, and legacy
  command behavior across protocol versions.
