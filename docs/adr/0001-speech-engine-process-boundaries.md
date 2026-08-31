# ADR 0001: Speech Engine Process Boundaries

- Status: Accepted
- Date: 2026-09-01

## Context

Omnivox can use operating-system speech services, open engines built as part of
a maintained payload, and optional third-party speech runtimes installed by a
user. These engines do not have the same architecture, licensing,
redistribution, reliability, or dependency characteristics.

Running every engine inside Omnivox gives the shortest call path, but a faulty
or blocking library can then stop the complete speech server. Some established
speech libraries are available only with a 32-bit ABI while the main Omnivox
process is 64-bit. Proprietary or separately licensed runtimes may also be
present on a user's machine without being suitable for inclusion in an
Omnivox release.

Putting every engine behind another process would instead add packaging,
protocol, lifecycle, and latency costs even for stable platform APIs and the
portable fallback engine. The project needs one consistent rule for deciding
whether an engine adapter runs inside Omnivox or in an isolated helper.

## Decision

### Define the boundary by process topology

A **built-in engine** has its adapter and engine calls in the Omnivox process.
The engine may be compiled into Omnivox or dynamically linked; built-in does
not mean statically linked. A detached or cancellation-aware in-process call
boundary does not turn an engine into a helper.

A **helper engine** runs a dedicated executable outside the Omnivox process
and communicates through the bounded, versioned
[engine helper protocol](../protocols/HELPER-PROTOCOL.md). Each unrelated
external runtime normally has its own helper executable, although helpers may
share protocol and support code.

Core, well-behaved platform or reproducibly controlled engines are eligible to
be built in. Optional, externally supplied, legacy, experimental, or
failure-prone engines use helpers by default.

### Treat some conditions as hard helper requirements

An engine must use a helper when any of the following applies:

- its library ABI has a different architecture from the Omnivox process;
- the user must supply the runtime and it cannot be an ordinary controlled
  Omnivox build dependency;
- its dependencies would conflict with dependencies in the main process;
- the API can hang, crash, corrupt global state, or otherwise requires failure
  isolation; or
- loading it in the main process would create an unacceptable security or
  lifecycle boundary.

Large optional runtimes, model-based engines, legacy global-state APIs, and
new integrations whose reliability is not yet established should also begin
as helpers. They may remain helpers even if none of the hard requirements is
permanent.

### Require evidence before building an engine in

An engine is eligible to run in the Omnivox process only when:

- its API is stable and supported on the target platform;
- it uses the same process architecture as Omnivox;
- the operating system supplies it, or its runtime and data can be built and
  distributed under understood terms;
- its dependencies are controlled and reproducible;
- failure is sufficiently contained that it does not put the complete speech
  service at unreasonable risk;
- in-process operation provides a useful simplicity, latency, streaming, or
  marker-accuracy benefit; and
- normal automated builds and release checks can exercise the integration.

Eligibility does not require an engine to be built in. Isolation may still
outweigh the in-process benefit.

### Standardize helper behavior

A helper for a user-supplied runtime contains Omnivox integration code, not the
external speech library. It discovers and dynamically loads an installed
library at run time. It must be able to start and report a useful
`runtime_unavailable` error when that library is absent rather than failing at
process load time.

An explicitly configured absolute library path has priority. Any automatic
discovery is limited to documented installation locations or platform
registration. A helper must not search the current working directory or use an
unrestricted library search path. It uses the platform's restricted absolute
loading mechanism and resolves or invokes native symbols only after validating
the selected library.

The helper validates architecture, required symbols, and any supported runtime
version before accepting synthesis. A 32-bit runtime uses a 32-bit helper even
when Omnivox is 64-bit; the protocol, rather than a shared address space,
bridges the architectures.

The helper reports availability, engine identity, voices, capabilities,
terminal errors, and cancellation state through the common protocol. It
normally returns PCM and timing events to Omnivox so the main server retains
control of mixing, ordering, selective cancellation, markers, and playback.
An engine that cannot do so requires a documented capability reduction or a
separate decision.

A missing, rejected, crashed, or timed-out helper makes only that engine
unavailable. Omnivox remains able to select a configured fallback and must not
present helper absence as failure of an otherwise working server.

### Keep process and distribution decisions distinct

Using a helper does not by itself decide whether its external runtime may be
distributed. For ECI, DECtalk, and initially RuTTS, Omnivox distributions ship
the compiled helper and applicable source and notices, while the user supplies
the speech library and its required voice, dictionary, or other runtime data.

An open helper runtime may be published as a separately reviewed companion, as
intended for Piper, only after its provenance, licence, corresponding-source
obligations, security updates, reproducible build, relocation, and notices
have passed the release gate. A helper boundary is not permission to copy or
redistribute a third-party runtime.

Process isolation is a technical boundary, not a legal conclusion. Every
engine integration still requires an appropriate licensing and provenance
review. The maintained component policy remains in
[LICENSING.md](../LICENSING.md).

### Apply the initial classification

The intended boundaries are:

| Engine | Boundary | Rationale |
|---|---|---|
| Windows WinRT SpeechSynthesizer | Built in | Supported operating-system API and core Windows engine. |
| macOS AVSpeechSynthesizer | Built in | Supported operating-system API and core macOS engine. |
| eSpeak NG | Built in | Open, reproducibly buildable portable fallback with controlled runtime data. |
| Piper | Helper | Optional model and native dependency stack with an independent lifecycle. |
| Freedom Scientific Eloquence/ECI | Helper | User-supplied proprietary library, 32-bit ABI, and native failure isolation. |
| Software DECtalk | Helper | User-supplied legacy runtime, dictionary data, 32-bit callback ABI, and native failure isolation. |
| RuTTS | Helper initially | External candidate whose ABI, packaging, licensing, and reliability require evaluation. |

This table records the process boundary, not an assertion that every listed
integration is implemented, published, or supported on every platform.

New external engines begin as helpers unless their proposal demonstrates the
built-in criteria above. Moving an established engine across the process
boundary changes packaging and failure behavior and requires this decision to
be revisited or superseded.

## Consequences

### Benefits

- Optional proprietary and legacy engines cannot crash the main speech server
  merely by being loaded into its address space.
- A helper can match a 32-bit or otherwise incompatible library independently
  of the main Omnivox architecture.
- Missing optional engines produce actionable diagnostics and do not prevent
  fallback speech.
- Engine adapters share one protocol, inventory, cancellation, testing, and
  capability model without combining unrelated runtimes in one helper.
- Packaging reviews remain explicit instead of being inferred from process
  topology.

### Costs and risks

- Each helper adds a process, executable, protocol lifecycle, packaging work,
  and failure reporting to maintain.
- Audio and marker transport across a process boundary can add latency and
  requires bounded buffering and cancellation behavior.
- Users must obtain separately supplied libraries and data themselves.
- Stub and source-contract tests cannot prove compatibility with every real
  runtime version or installation.
- A separately published open-runtime companion needs its own reproducible
  build, provenance, licensing, and verification work.

## Alternatives considered

### Build every engine into Omnivox

Rejected. Architecture mismatches make this impossible for some libraries,
and one optional engine could destabilize the complete speech service.

### Put every engine behind a helper

Rejected. It adds unnecessary process and protocol machinery for stable
operating-system APIs and the controlled portable fallback.

### Use one universal external-engine helper

Rejected as the default. It would put unrelated dependencies and failure
domains back into one process and complicate architecture-specific builds.
Shared protocol and support code provide reuse without sharing the runtime
process.

### Package any runtime that a helper can load

Rejected. Technical compatibility does not establish provenance or
redistribution permission, and it obscures which project is responsible for
runtime security and updates.

### Decide separately for every engine without a common policy

Rejected. Ad hoc decisions produce inconsistent packaging and make future
engines harder for users, contributors, and release maintainers to evaluate.

## Follow-up

1. Audit the existing Eloquence and DECtalk helpers against the safe dynamic
   loading, architecture validation, diagnostic, and fallback requirements.
2. Keep the common helper protocol and contract tests authoritative for
   discovery, capabilities, synthesis, cancellation, errors, and process
   failure.
3. Retain source-contract and stub testing without proprietary runtimes, with
   opt-in acceptance and stress tests for legally installed real runtimes.
4. Complete Piper's separate companion-release gates before treating a build
   candidate as a published runtime.
5. Evaluate RuTTS through an isolated helper before considering it a supported
   engine.
6. Keep current implementation facts in
   [ARCHITECTURE.md](../ARCHITECTURE.md), platform claims in
   [STATUS.md](../STATUS.md), and runtime supply terms in
   [LICENSING.md](../LICENSING.md).

## Implementation status

WinRT, AVSpeechSynthesizer, and eSpeak NG currently use the built-in boundary.
Piper, Eloquence/ECI, and DECtalk currently use the helper protocol. The
existing Windows helpers and their user-supplied runtime boundary are described
in the [Windows helper guide](../../windows-helpers/README.md). RuTTS remains an
evaluation candidate rather than an implemented engine.
