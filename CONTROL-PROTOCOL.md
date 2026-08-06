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
  currently implements capability negotiation and active-engine inventory;
  logical-voice registration will extend the version without changing its
  envelope.

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

The initial error codes are:

- `malformed_request`: invalid Base64, UTF-8, JSON, or request shape;
- `unsupported_version`: a well-formed request uses an unsupported version;
- `payload_too_large`: the encoded or decoded size bound was exceeded.

If decoding fails before the request ID can be trusted, `request_id` is `null`.
Errors do not mutate speech, queue, engine, or logical-voice state.

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
