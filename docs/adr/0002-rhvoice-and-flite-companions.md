# ADR 0002: RHVoice and Flite Companion Policy

- Status: Accepted
- Date: 2026-09-01

## Context

ADR 0001 requires new external speech engines to begin behind dedicated helper
processes. RHVoice and Flite have different runtime, data, licensing, and
platform characteristics, so using the same process boundary does not imply
using the same distribution policy.

RHVoice provides a callback-based native API with streamed PCM, cancellation,
word and sentence events, physical voice discovery, and rate, pitch, volume,
punctuation, character, and key controls. Its upstream project supports
GNU/Linux, Windows, and Android. The desktop runtime and its language and voice
data are installed separately, and individual voice licences vary.

Flite is a small portable ANSI C engine with a BSD-like runtime licence. Its
latest tagged release is v2.2 at commit
`e9e2e37c329dbe98bfeb27a1828ef9a71fa84f88`. Upstream publishes source rather
than maintained binary packages for every Omnivox target. Flite can compile
voices into the runtime or load external `.flitevox` files.

## Decision

### Keep both engines in dedicated helpers

RHVoice and Flite each use their own executable and the common bounded helper
protocol. The helpers share protocol-host support code but never share a
runtime process. A crash, hang, global-state fault, or unavailable dependency
therefore removes only the affected engine.

The common protocol host is an engine-neutral workspace crate. Engine-specific
adapters and native dependencies remain outside it.

### Load RHVoice from the user's installation

Omnivox does not redistribute RHVoice libraries, language data, or voice data.
The RHVoice helper dynamically loads a compatible user-installed runtime using
the restricted absolute-path rules in ADR 0001. An explicit library path has
priority over documented installation locations. Data, configuration, and
resource path overrides are passed to RHVoice's public initialization API.

The adapter accepts the stable C API shared by RHVoice 1.14.0 through 1.18.4,
validates all required symbols, and reports the loaded runtime version. It uses
native callbacks for PCM, cooperative cancellation, word and sentence markers,
and completion. Missing runtime or voice data is reported as engine
unavailability without preventing Omnivox from starting with another engine.

GNU/Linux and Windows are supported where an upstream-compatible RHVoice
runtime is available. macOS and Windows ARM64 builds may carry the helper and
explicit path overrides, but are not described as live-runtime support until a
compatible runtime passes synthesis and cancellation acceptance on that target.

### Publish Flite as a separate source-built companion

The Flite helper is built reproducibly from the pinned v2.2 source and linked
only into its own process. The companion contains `cmu_us_slt` as its sole
compiled-in voice. A user may opt into additional local `.flitevox` voices;
Omnivox does not download voice files at runtime.

The Flite companion remains separate from generic Omnivox archives. Its
release gate records the upstream source hash, complete licence text, build
inputs, notices, corresponding source, relocation checks, and real synthesis.
The target matrix is:

| Platform | Architecture |
|---|---|
| GNU/Linux | x86-64, ARM64 |
| macOS | Intel, Apple Silicon |
| Windows | x86-64, ARM64 |

Flite is an English compact fallback and does not replace eSpeak NG as the
Unicode-capable final fallback.

### Keep platform claims evidence-based

Every target compiles and runs source-contract and protocol tests. A platform
is described as live-runtime supported only after native voice discovery,
synthesis, PCM validation, cancellation or helper retirement, and clean
shutdown pass there. Compile-only or stub coverage is labelled accordingly.

## Consequences

- RHVoice users retain control over runtime and voice installation and the
  licences that apply to their selected voices.
- Omnivox needs documented RHVoice discovery for each supported desktop
  platform and real-runtime acceptance separate from ordinary CI.
- Flite users receive one known compact voice without turning every generic
  Omnivox archive into a native-runtime bundle.
- Six Flite companion builds and a corresponding-source artifact become part
  of release maintenance.
- Extracting the engine-neutral helper host avoids copying lifecycle,
  cancellation, bounds, and protocol behavior into every native helper.

## Alternatives considered

### Require system installations for both engines

Rejected. It would keep release contents small but make Flite difficult to
install consistently on Windows and macOS and would weaken reproducible
cross-platform acceptance.

### Bundle both runtimes and their voices

Rejected. RHVoice voice licences vary, its official platform coverage does not
match every Omnivox target, and bundling its runtime and data would add a much
larger corresponding-source and packaging surface.

### Build either engine into the main process

Rejected for the initial integrations. RHVoice is user-supplied and Flite is a
new native/global-state integration. Both therefore meet ADR 0001's default
helper criteria, and the small latency benefit does not yet outweigh failure
isolation.
