# ADR 0001: Engine isolation and distribution

- Status: Accepted
- Consolidated: 2026-09-27 from the existing accepted process and companion policies.
- Related: [Progressive synthesis](0003-progressive-audio-and-markers.md),
  [managed voices](0007-managed-voice-lifecycle.md).

## Context

Operating-system speech services, controlled open engines and user-installed
native runtimes have different ABI, reliability, dependency and distribution
constraints. Optional native failures must not disable the whole speech service.
Process topology and permission to distribute a runtime are independent choices.

## Decision

### Choose the process boundary explicitly

A built-in engine calls its runtime inside Omnivox, whether statically or
dynamically linked. A helper runs a dedicated executable using the bounded,
versioned [helper protocol](../protocols/HELPER-PROTOCOL.md). A detached native
call inside Omnivox is not a helper boundary.

A helper is required for a different-architecture ABI, a user-supplied runtime
that is not a controlled build dependency, conflicting dependencies, unsafe
global state, or native calls whose failure threatens the server. Optional,
experimental, large or unqualified engines begin as helpers. Each unrelated
runtime has its own process; adapters may share protocol support code.

In-process eligibility requires a stable same-architecture API, controlled
dependencies, understood runtime/data terms, adequate fault containment, a
useful latency or simplicity benefit, and normal build/release verification.
WinRT, AVSpeechSynthesizer and the controlled eSpeak NG fallback retain this
boundary. Moving another engine in-process requires a new architectural decision
and evidence; being small or open source is insufficient.

### Load user runtimes through a live helper

A helper for a user-installed runtime contains integration code and dynamically
loads the library after starting its protocol host. Missing or rejected native
files produce useful `not_available` errors rather than a process-load failure.
An explicit absolute library path takes priority; discovery is restricted to
documented installation locations or platform registration, never the working
directory or an unrestricted library path. Validate architecture, required
symbols and supported versions before native calls. A 32-bit runtime uses a
32-bit helper even with a 64-bit server.

The adapter owns native initialization, serialization, text encoding and cleanup.
It rejects unrepresentable text rather than silently replacing characters.
It returns PCM and truthful timing/capability metadata to Omnivox, which owns
mixing, effects and playback. Direct external playback requires an explicit
capability reduction or another decision. Common framing, bounded cancellation,
cleanup and recovery remain in the shared host. An unavailable optional engine
leaves eligible ordinary fallback speech available.

### Preserve component-specific supply policies

| Component | Accepted runtime/distribution boundary |
| --- | --- |
| Eloquence/ECI and DECtalk | Maintained separate helpers; users supply compatible libraries and required dictionaries/data. Windows helpers bridge the x86 ABI. Preserve their GPL-2.0-or-later source/notices and matching source/build material in applicable distributions. |
| RHVoice | Separate helper dynamically loads the user's compatible 1.x C API runtime. The runtime is not redistributed by Omnivox. Data may be installed externally or acquired through the explicit managed provider; neither runtime nor voice assets join executable releases. |
| Flite | Separate reproducible companion built from pinned v2.2, with `cmu_us_slt` as the sole compiled voice. Additional `.flitevox` files are explicit imported/managed inputs. It is an English fallback and does not replace eSpeak's Unicode fallback role. |
| RuTTS | Separate companion built from pinned v6.3.3 with both built-in voices. KOI8-R conversion and signed 8-bit native PCM remain adapter concerns. No RuLex library or database is included or loaded; adding RuLex requires its own provenance and architecture review. |
| TGSpeechBox | Separate experimental pinned beta companion. The helper contains the eSpeak IPA frontend and native DSP; its combined binary is GPL-3.0-or-later while the narrow Omnivox boundary and upstream TGSpeechBox source retain their own notices. Windows x64 GNU is the release-qualified target; generic archives and the Emacsvox release bundle exclude it. |
| Piper | Separate optional companion with its own runtime/dependency and corresponding-source gates. Voice models remain outside executable releases, including the CI-only test model. |
| MBROLA | Explicit absolute `OMNIVOX_MBROLA_HELPER` opt-in to the private development companion. Keep the AGPL runtime, GPL frontend and database terms distinct; managed English databases do not promote it into a supported generic artifact or imply macOS acceptance. |

Companion publication requires pinned inputs, source/archive verification,
complete component notices, corresponding source, reproducible builds,
relocation and native synthesis/cancellation/shutdown gates. Flite and RuTTS
target Linux, Windows and macOS on x64 and ARM64; a target is runtime-supported
only after its native acceptance passes. RHVoice compilation on a platform
without a qualified runtime remains compile-only. Engine caching/prewarming
cannot substitute unverified descriptors for the selected live helper.

The exact source locks and operational instructions belong to the
[component licensing map](../LICENSING.md) and linked engine guides. A helper
boundary does not grant redistribution rights or authorize engine-side downloads.
Managed acquisition is a separate local service under
[ADR 0007](0007-managed-voice-lifecycle.md).

## Consequences and alternatives

Isolation contains crashes, incompatible ABIs and blocked native work, at the
cost of extra processes, packaging and transport. Ordinary protocol tests can
use fake runtimes; real platform/runtime and audible acceptance remain distinct.

Building every engine in-process is incompatible with the required failure and
ABI boundaries. Putting every platform API behind a helper adds unnecessary
machinery. A universal helper hosting unrelated runtimes recreates dependency
and failure coupling. Automatically redistributing anything a helper can load
would conflate technical compatibility with the component supply policy.
