# Local audio-device selection

This is the server contract under [ADR 0020](../adr/0020-audio-output-device-selection.md).
Client adoption, full runtime staging and platform/listening qualification are
tracked separately in the [delivery plan](../plans/audio-device-selection.md).

## Scope and startup

`audio_device_selection_v1` is advertised by ordinary local Windows workers using
the `device` backend. It is absent for remote workers, emergency workers, null,
native PulseAudio and other platforms. Negotiate it separately on each worker;
unsupported peers reject the operations without changing output.

`--list-audio-devices` emits one JSON catalogue without constructing speech
engines or reading engine configuration. Unsupported platforms return
`supported: false` and an empty list. It enumerates the Windows device backend,
not the devices behind a Linux WSLg virtual output.

`--audio-device default` follows the system default. Otherwise the value is an
opaque backend-qualified ID returned by discovery. The CLI overrides
`OMNIVOX_AUDIO_DEVICE`, then the matching saved client choice below; omission
preserves follow-default behavior.
Explicit IDs require Windows `device` output and never select a fallback device.
The same selection applies to server output, `--check` and file playback.
Device IDs are not new members of configuration version 2's `audio` object.
Clients save host-scoped lane choices and supply their launch override explicitly.

`OMNIVOX_AUDIO_DEVICE_PREFERENCES` is a client projection for **one lane**: a JSON
array of `{host_scope, selection}` records. It contains at most eight distinct
hosts and 16,384 UTF-8 bytes; host scopes contain 1–512 bytes without control
characters. Unknown fields, repeated hosts and duplicate JSON keys are errors.
Every selection is validated, even for another host. A nonmatching host uses
system default. The worker resolves its own scope during normal startup; the
client need not run an extra discovery process. CLI/environment overrides take
precedence over this projection, including an explicit `default`.

## Requests

Use the existing version-1 Base64 JSON control envelope. Every new request
rejects unknown and duplicate keys, including nested selection/change objects.
Neither discovery nor Apply accepts a path or executable definition.

```json
{"protocol_version":1,"request_id":1,"type":"audio_devices_v1"}
{"protocol_version":1,"request_id":2,"type":"audio_device_status_v1"}
{"protocol_version":1,"request_id":3,"type":"apply_audio_device_v1","incarnation":"WORKER_UUID","change":{"sequence":1,"expected_revision":0,"selection":{"mode":"device","id":"wasapi:ENDPOINT_ID"}}}
{"protocol_version":1,"request_id":4,"type":"cancel_audio_device_v1","incarnation":"WORKER_UUID","sequence":1}
```

A selection is exactly `{"mode":"system_default"}` or
`{"mode":"device","id":"BACKEND:OPAQUE_ID"}`. IDs contain nonempty backend and
device parts, no control characters, and at most 1,024 UTF-8 bytes. Friendly names
and list positions never identify devices. An unknown/incompatible ID is an
error, not a request to choose the closest name.

## Discovery and state

`audio_devices_v1` replies contain `host_scope`, `backend`, `supported`, nullable
`default_device`, `devices` and `truncated`. Each device is `{id, name}`. The list
contains at most 64 outputs; labels are shortened on a UTF-8 boundary to at most
512 bytes. Equal labels remain separate entries ordered by ID. Enumeration can
fail or become stale while a device changes; refresh explicitly to retry.

Windows `host_scope` is `windows:wasapi:` followed by its lowercase physical DNS
host name. Scope saved choices to that host/backend, not a launcher path or
Omnivox build. A renamed host has a new scope. The scope is configuration
namespacing, not authentication or an assertion that names are globally unique.

`audio_device_status_v1` returns `incarnation`, `host_scope` and `state`. The
incarnation is a UUID generated for this worker's output management lifetime.
State contains:

| Field | Meaning |
| --- | --- |
| `revision` | Unsigned 64-bit explicit-selection revision, initially zero. |
| `selection` | Confirmed default/explicit selection. |
| `connected` | Actual endpoint ID, or null while unavailable. |
| `last_operation` | Null or `{change, outcome, reason}`; reason is nullable. |

State reads perform no native device or engine calls. Automatic reconnect can
change `connected` without changing the selection revision. Availability is an
observation, not a promise that the device cannot disconnect immediately after it.

## Apply, cancellation and uncertain replies

Apply/cancel replies use `audio_device_result_v1` with `sequence`, `outcome` and
`status` (the complete status above). Outcomes are `preparing`, `applied`,
`unchanged`, `rejected`, `cancelled`, `busy` or `stale`. Query status until the
matching operation is terminal. A timeout is unknown; it is not success, automatic
cancellation or permission to roll back.

Sequences start at one and increase by exactly one from the last accepted
operation. Apply requires the current incarnation and revision. Repeating the
same change returns its retained outcome; reusing a sequence with different
contents is stale. Concurrent preparation is busy. Rejected, cancelled and
unchanged accepted operations consume their sequence. Successful publication
increments the revision; unchanged selections do not. Identities never wrap.
Only the latest accepted operation is retained. Old replies cannot establish the
state of a newer incarnation or supersede a later operation.

Preparation opens the exact candidate on the existing output-owner thread while
the previous connection keeps playing. Failure/cancellation before publication
preserves that working route. Cancel identifies the operation by incarnation and
sequence; once publication has happened it reports the terminal outcome and
cannot undo it. Native preparation retains ownership until its eventual return.

Publication rechecks device notifications and request state, then retires all
three old queues, overlays and the previous synthesis generation. Engines and
client speech settings stay intact. Old speech and unreached cues never replay.
An explicit device ignores unrelated default changes. If it disappears, discard
new audio while unavailable and retry only that ID; choosing default or another
device requires explicit Apply. Initial unavailable output remains a startup
error. Main and notification operations are independent, with separate results.

Discovery has one working request and one queued request. Replies use a bounded
16-record background queue. A blocked reader can lose a reply without blocking
Stop in these handlers. It must query the existing operation rather than infer an
outcome or retry it with a new sequence. Existing synchronous operations have
their own limits; this does not change every control-response path.

## Retained recovery

A client retains only confirmed applied choices with the lane's launch context.
For an owned replacement it supplies `OMNIVOX_OWNED_AUDIO_DEVICE` and
`OMNIVOX_OWNED_AUDIO_DEVICE_HOST` explicitly. The owner requires its actual host
scope to match, validates the bounded selection, replaces only its audio-device override,
keeps the frozen engine snapshot intact and saves the resulting complete owner
record before starting the worker. This does not reread edited engine settings.
The actual route must be established before startup audio or ordinary speech.
When sharing an engine record between lanes, replace its lane audio projection
and remove inherited device arguments before applying the destination lane's
override; the main lane's device must not become the notification default.

Clients must reconcile an unresolved Apply before automatic replacement; if the
old worker cannot report its outcome, report the uncertainty instead of assuming
the old route. A later explicit choice may establish the replacement destination,
subject to existing verified native retirement. Emergency startup discards these
ordinary device preferences and retains its explicitly selected output backend.
