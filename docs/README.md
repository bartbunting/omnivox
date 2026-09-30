# Omnivox Documentation

Start with the repository [README](../README.md) for installation and first use.
This index separates current instructions, exact contracts, proposed work and
retained evidence. The [documentation guide](DOCUMENTATION-GUIDE.md) defines
document responsibilities and maintenance rules.

## Current behavior and design

- [Architecture](ARCHITECTURE.md): runtime components, ownership, routing,
  cancellation, audio and failure handling.
- [Status](STATUS.md): implemented capabilities, platform qualification and limits.
- [Roadmap](ROADMAP.md): outstanding work and links to active proposals.
- [Licensing](LICENSING.md): component supply and distribution boundaries.

## Architecture decision records

The [ADR index](adr/README.md) records durable choices and distinguishes accepted
decisions from proposals. Accepted design, implementation, platform qualification
and release publication are different states. Engine registration version 1 is
implemented; the [roadmap](ROADMAP.md#extensible-engine-registration) tracks
later extensions and remaining platform qualification.

## Operations and releases

| Task | Guide |
| --- | --- |
| Configure a speech host | [CLI, environment and adapters](guides/configuration.md) |
| Change punctuation names | [Punctuation configuration](guides/configuration.md#punctuation-pronunciations) and [two-stage plan](plans/punctuation-configuration.md) |
| Add an independently installed engine | [Helper registration walkthrough](guides/configuration.md#add-your-own-speech-engine) |
| Diagnose speech failures | [Logs, lifecycle tracing and crash dumps](guides/diagnostics.md) |
| Manage downloaded/imported voices | [Installation, enablement, Apply/rollback and removal](guides/voice-management.md) |
| Validate native voice inputs | [Disposable checks, saved evidence and operation commands](guides/native-voice-validation.md) |
| Use workstation speech remotely | [Preview setup, SSH and recovery](guides/remote-speech.md) |
| Install/build Debian packages | [Debian packages](guides/debian-packages.md) |
| Compare Windows/Linux speech under WSLg | [Development comparison workflow](guides/wsl-audio-comparison.md) |
| Build, verify or release payloads | [Deployment](../.github/DEPLOYMENT.md), [workflow reference](../.github/workflows/README.md), [developer tools](../tools/README.md) |

Published history remains in the [changelog](../CHANGELOG.md).

## Engine guides

Keep one guide per engine or distinct native feature. Each identifies runtime
supply, configuration, verification and platform limits.

| Engine | Guide |
| --- | --- |
| Piper | [Companion, models and release maintenance](engines/piper.md) |
| RHVoice | [User runtime and external/managed data](engines/rhvoice.md) |
| Flite | [SLT companion and external voices](engines/flite.md) |
| RuTTS | [Russian companion and text repertoire](engines/rutts.md) |
| TGSpeechBox | [Experimental formant companion](engines/tgspeechbox.md) |
| MBROLA | [Explicit development companion](engines/mbrola.md) |
| eSpeak NG variants | [On-demand combinations and exact identity](engines/espeak-variants.md) |
| macOS voices | [Native streaming and verification](engines/macos.md) |
| Eloquence and DECtalk | [Windows helpers](../windows-helpers/README.md), [Linux helpers](../linux-helpers/README.md) |

The platform helper build READMEs remain with the code they maintain.

## Protocol specifications

- [Legacy line protocol](protocols/legacy.md): Emacspeak command grammar and state.
- [Control protocol](protocols/control.md): discovery, inventory, routing,
  previews, completion and marker events.
- [Presentation timeline](protocols/presentation-timeline.md): structured spans,
  multipart transport, actions and degradation.
- [Engine helper protocol](protocols/helper.md): isolated synthesis processes.
- [Remote protocol](protocols/remote.md): authenticated preview transport.
- [Voice-choice tuning](protocols/voice-choice-tuning.org): authoritative layered
  composition and compatibility across protocol versions.
- [Native voice parameters](protocols/engine-voice-parameters.md): authoritative
  native catalogue, composition, preview and evidence contract.
- [Protocol fixtures](protocol-fixtures/): executable examples consumed by Rust
  tests. Their paths and wire contents are independent of prose reorganization.

## Implementation and format references

- [Native-call isolation](reference/native-call-isolation.md): quarantine and
  bounded capacity for uncancellable native calls.
- [Prepared synthesis](reference/prepared-synthesis.md): attempts, transactional
  audio commitment, effects, tickets and consumption evidence.
- [Text chunking](reference/text-chunking.md): preprocessing and original offsets.
- [Engine configuration](reference/engine-configuration.md): version-1 helper
  manifests, local policy, overrides and startup rules.
- [Rate calibration](reference/rate-calibration.md): mappings, measured reference
  curves, reproduction and interpretation.
- [Voice-library formats](reference/voice-library.org): storage, identity,
  eligibility, generations and activation.
- [Operation journals](reference/voice-operation-journals.md): persistent
  ownership, transitions, cleanup and recovery.
- [Validation evidence](reference/validation-evidence.md): saved observations,
  comparison and publication limits.

## Evidence

Use the [evidence index](benchmarks/README.md) to choose a matched performance
baseline or find native/functional acceptance. Benchmark packs, experiment
reports and [rate audits](rate-audits/README.md) retain their existing paths,
raw samples, reproduction inputs and provenance. Functional pass counts do not
establish performance or audible acceptance. Reruns create new reports.

## Active proposals

The [roadmap](ROADMAP.md) is the entry to future work. Current detailed proposals:

- [Language-aware voice selection](plans/language-routing.md): explicit language
  matching and fallback beyond the existing exact-match rules.
- [Speech Dispatcher feasibility](plans/speech-dispatcher.md): external playback,
  capability reductions and completion questions before implementation.

Completed plans contribute lasting requirements to guides/references and results
to evidence, then leave the working tree. Git retains the implementation history.
