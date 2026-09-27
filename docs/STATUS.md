# Omnivox Project Status

**Documentation reviewed:** 2026-09-27. **Workspace version:** 1.12.0.
This review adds no native qualification. Published changes belong to the
[changelog](../CHANGELOG.md); current implementation may include unreleased work.
[Architecture](ARCHITECTURE.md) explains behavior, [the roadmap](ROADMAP.md)
tracks outstanding work, and [retained evidence](benchmarks/README.md) records
actual runs and their limits.

## Implemented

| Area | Current capability | Maintained reference |
| --- | --- | --- |
| Admission | Bounded legacy/control input, atomic multipart timelines, generation-safe queue admission and cancellation. | [Legacy](protocols/legacy.md), [control](protocols/control.md), [timeline](protocols/presentation-timeline.md) |
| Routing | Stable physical identities, ordered logical choices, runtime fallback, exclusions, health circuits and asynchronous helper recovery. | [Architecture](ARCHITECTURE.md#voice-identity-routing-and-tuning) |
| Voice customization | Shared → actual choice → context composition; private exact/full previews; typed qualified native controls. Control envelope 1, timeline versions 1–5, markers 1–4 and helper versions 1–6 retain compatibility. | [Layered tuning](protocols/voice-choice-tuning.org), [native parameters](protocols/engine-voice-parameters.md) |
| Discovery | Concurrent helper initialization before initial inventory, verified bounded caches and on-demand exact eSpeak variants. | [Architecture](ARCHITECTURE.md#configuration-and-engine-discovery), [variants](engines/espeak-variants.md) |
| Engine configuration v1 | Strict local `config.json`/helper manifests, immutable launch snapshots, local permissions, paired startup/Apply acknowledgements and remote session retention. Blocked startup retains ownership without delaying admission; full development acceptance remains incomplete. | [Configuration](guides/configuration.md), [integration evidence](benchmarks/2026-09-27-engine-framework.md), [deadline fix](benchmarks/2026-09-27-engine-startup-deadline.md) |
| Managed voices | Reviewed Piper, Flite, MBROLA and RHVoice acquisition; disabled installation, immutable generations, explicit two-worker Apply/rollback and reviewed removal. | [Voice management](guides/voice-management.md), [formats](reference/voice-library.org) |
| Native validation | Disposable bounded native probes, before/after evidence, ownership journals and recorded-cleanup recovery. | [Validation](guides/native-voice-validation.md) |
| Synthesis | Buffered and bounded progressive PCM, source-mapped anchors, actual-attempt settings and no replay after audio commitment. | [Helper protocol](protocols/helper.md), [prepared synthesis](reference/prepared-synthesis.md) |
| Presentation | Canonical stereo 44.1 kHz PCM, trimming, effects, inserted/overlaid resources, independent speech/tone/sound streams and tracked source completion. | [Architecture](ARCHITECTURE.md#audio-and-presentation-ownership) |
| Output | Default device output, explicit null diagnostics, and opt-in native PulseAudio on Linux. | [WSLg comparison](guides/wsl-audio-comparison.md), [ADR 0005](adr/0005-native-pulseaudio-output.md) |
| Remote speech | Preview authenticated loopback service over SSH forwarding; independent foreground/notification workers and reconnect without replay. | [Remote setup](guides/remote-speech.md) |
| Diagnostics | Correlated admission/synthesis/playback records, sensitive text opt-in and optional Windows crash dumps. | [Diagnostics](guides/diagnostics.md) |

## Platform and CI coverage

| Platform | Qualified core / optional-runtime boundary | Generic release artifact |
| --- | --- | --- |
| macOS ARM64 and x64 | AVSpeechSynthesizer and eSpeak; native Piper, Flite and RuTTS companion gates. Streaming native/null-output checks passed on both architectures; audible Emacs streaming acceptance remains open. | Yes; core Homebrew installation also available |
| Windows x64 | WinRT and eSpeak; RHVoice runtime acceptance; Piper, Flite, RuTTS, user-runtime Eloquence/DECtalk and experimental TGSpeechBox have their respective native checks. | Yes |
| Windows ARM64 | WinRT and eSpeak; Flite/RuTTS companion gates. Do not infer qualification of other runtimes from x64 results. | Yes |
| Linux x64 | eSpeak; RHVoice runtime acceptance; Piper, Flite and RuTTS companions. ECI/DECtalk and MBROLA retain development qualification below. | Yes; Ubuntu 24.04 ABI baseline |
| Linux ARM64 | Flite/RuTTS companion gates; RHVoice helper compile coverage. No generic-server runtime/release job. | No |

Generic native release jobs verify relocated draft assets and eSpeak synthesis;
Windows and macOS also verify their system engine. Debian amd64 packages are
checked on Ubuntu 24.04 and 26.04. Release binaries remain unsigned. Exact gates,
source requirements and installation instructions belong to the
[deployment guide](../.github/DEPLOYMENT.md), [Debian guide](guides/debian-packages.md) and
[licensing map](LICENSING.md).

Core Homebrew installation, reinstall, packaging-revision upgrade and removal
passed on Intel and Apple Silicon. A real upgrade between upstream versions and
audible Emacs use remain outstanding. [Retained platform results](benchmarks/2026-09-27-retained-platform-results.md)
include the source and CI identities; formula updates remain explicit.

## Engine qualification and limits

| Engine | Evidence and limits |
| --- | --- |
| eSpeak NG | Packaged discovery and synthesis on generic release targets; progressive PCM and source-accurate anchors. Variant checks use actual native identities; null output does not prove listening quality. |
| WinRT | Native Windows release checks. Uncancellable calls may continue quarantined after their stale PCM is suppressed. |
| macOS | Bounded callback streaming on Intel/Apple Silicon; no native markers. Rate remains system-native pending calibration. See [macOS guide](engines/macos.md). |
| Piper | Relocated companions and real CI-model synthesis on Linux x64, Windows x64 and both Macs. Models remain separate; one resident model per helper. No native markers. See [Piper](engines/piper.md). |
| RHVoice | Real 1.14.0 C API synthesis, markers, ACSS, cancellation and shutdown on Linux/Windows x64. Compatible 1.14+ 1.x runtime remains user-installed; macOS is compile-only and Windows ARM64 has no accepted runtime. See [RHVoice](engines/rhvoice.md). |
| Flite | Native six-target companion gates, 25 SLT syntheses, relocation, ACSS, cancellation and shutdown. ASCII guarantee and word-boundary anchors; compatible external English voices only. See [Flite](engines/flite.md). |
| RuTTS | All six companion release gates passed for v1.7.1; additional Windows GNU/Linux development evidence. KOI8-R repertoire, two built-in voices, no RuLex or markers. Multi-hour memory and high-rate intelligibility need broader measurement. See [RuTTS](engines/rutts.md). |
| TGSpeechBox | Windows x64 GNU experimental companion; Linux x64 smoke checks. Exact requested anchors at the default progressive 44.1 kHz rate; 22.05 kHz remains buffered. Not included in generic/Emacsvox archives. See [TGSpeechBox](engines/tgspeechbox.md). |
| Windows Eloquence/DECtalk | User-supplied x86 runtimes; qualified helper-6 native controls and progressive markers. Qualification is runtime-specific. See [Windows helpers](../windows-helpers/README.md) and [native evidence](benchmarks/README.md#functional-acceptance-and-additional-reports). |
| Linux ECI/DECtalk | Development checks for installed Voxin 3.4 and x64 DECtalk 4.99, including all 17 voices, ACSS, markers and local navigation. Other runtime/architecture combinations and rate calibration remain unverified. See [Linux helpers](../linux-helpers/README.md) and [parity evidence](experiments/2026-09-07-linux-helper-parity.md). |
| MBROLA | Explicit private development helper with Linux/native-Windows acquisition, synthesis and paired Apply/rollback evidence. No macOS or release qualification; no streaming or markers. See [MBROLA](engines/mbrola.md). |

## Current limitations

- Engine configuration v1 passes expanded Linux and native Windows process
  acceptance. Windows capture now omits hidden drive-directory bookkeeping, and
  historical owner records retain package references through retirement; see
  the [Windows evidence](benchmarks/2026-09-28-windows-engine-snapshot.md).
  The complete framework acceptance audit and broader platform qualification
  remain pending.
  The earlier startup-deadline ownership gap has been
  [fixed and regression-tested](benchmarks/2026-09-27-engine-startup-deadline.md).
- Speech Dispatcher is unimplemented.
- Language selectors match exact case-insensitive tags. General language-range
  matching, automatic detection and language-preserving global fallback are not
  implemented; multilingual native coverage remains incomplete.
- Marker precision and cancellation strength vary by engine. Operations needing
  the whole waveform or unsupported anchors can remain buffered. Immediate legacy
  speech and letter commands use global policy rather than a named logical voice.
- Device output uses the pinned Rodio/CPAL stack. Public device selection,
  multi-device routing and native PipeWire output are not implemented. Channel
  routing selects left/right/both within one device. Native PulseAudio remains
  opt-in; it cannot repair a stalled shared WSLg/RDP server.
- Remote speech remains single-session preview. Native TLS, automatic tunnels,
  shared multi-user output and arbitrary remote resource uploads are absent.
- Managed runtime availability and platform acceptance vary by provider. A
  generation acknowledgement does not prove residency, audible identity or
  memory savings. Incomplete cleanup blocks conflicting work. Stronger
  power-loss recovery and interrupted-Apply reconciliation remain outstanding.
- Native macOS removal and broader live-client/audible managed-voice acceptance
  remain open. Historical Windows GNU component success is distinct from full
  server/companion and MSVC acceptance; later combined Windows runs qualify only
  the exact scenarios in their reports.
- Normalized rates are approximate across voices. Several engines saturate before
  Eloquence; Linux ECI/DECtalk and macOS still need native rate audits.

## Validation

Use `make fmt-check`, `cargo test --locked --workspace` and applicable Clippy
checks for runtime changes. Real helper/device behavior also needs its native
runtime and relevant platform checks. The [tools guide](../tools/README.md)
documents benchmark, stress and soak harnesses; the [evidence index](benchmarks/README.md)
selects matched baselines and retains raw data and provenance.

Mixer-source consumption, null-output completion, callback success and process
liveness are not acoustic onset or stop-to-silence measurements. Test pass counts
are historical observations, not performance thresholds. Remaining physical
output and long-session work is tracked in [the roadmap](ROADMAP.md).
