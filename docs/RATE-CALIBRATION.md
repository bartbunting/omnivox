# Speech-rate Calibration

Omnivox's host rate is a normalized routing control, not a native engine
parameter. Rate `0.5` targets the established Eloquence `v1` reference speed.
Engine adapters use measured piecewise curves so changing a logical voice does
not also cause a large, avoidable speed change.

## Reference measurements

The English calibration used the standard corpus in
`tools/audit_speech_rates.py` and these reference voices:

| Engine | Reference voice |
|---|---|
| Eloquence | `v1` (compatibility anchor) |
| Windows WinRT | Microsoft David, `en-US` |
| DECtalk | Perfect Paul |
| eSpeak NG | English (America) |
| RHVoice | SLT |
| Flite | built-in `cmu_us_slt` |
| Piper | locked CI-only `en_US-kristin-medium` model |

RuTTS was measured with both built-in voices and a 17-word Russian corpus.
Calibrated eSpeak Russian is its same-language anchor through host rate `0.8`.

The 2026-09-02 post-calibration Windows acceptance, together with the native
Linux Piper run, produced these canonical WPM values. Eloquence is the
unchanged target row:

| Engine | `0.3` | `0.4` | `0.5` | `0.6` | `0.7` | `0.8` |
|---|---:|---:|---:|---:|---:|---:|
| Eloquence `v1` | 120.2 | 151.8 | 186.5 | 235.2 | 287.2 | 369.0 |
| WinRT David | 121.3 | 152.2 | 184.8 | 236.6 | 285.2 | 373.2 |
| DECtalk Paul | 120.0 | 151.8 | 185.7 | 234.7 | 288.1 | 366.1 |
| eSpeak English (America) | 120.3 | 151.2 | 185.2 | 235.0 | 286.7 | 332.1 |
| RHVoice SLT | 120.0 | 151.7 | 186.4 | 234.6 | 257.9 | 258.0 |
| Flite SLT | 119.3 | 151.4 | 185.7 | 233.1 | 284.2 | 364.0 |
| Piper Kristin | 121.3 | 155.9 | 187.5 | 230.6 | 288.4 | 297.4 |

The portable eSpeak and Flite results reproduced across the Linux and Windows
payloads. Piper was measured natively on Linux with the locked CI model. Its
voice model, eSpeak, and RHVoice show the documented high-rate saturation.

The equivalent Russian acceptance illustrates the shared-language anchor:

| Engine | `0.3` | `0.5` | `0.8` |
|---|---:|---:|---:|
| eSpeak Russian | 142.5 | 195.5 | 365.4 |
| RuTTS male | 142.8 | 196.9 | 351.9 |
| RuTTS female | 142.0 | 191.4 | 386.6 |

The audit measures duration in the canonical post-pipeline WAV. It does not
measure process startup, helper startup, model loading, playback latency, or
real-time synthesis throughput. The current one-shot diagnostic starts a new
process for every sample; that costs audit time but cannot change the duration
stored in the WAV.

## Reproducing an audit

Build a complete runnable payload first. On the current platform:

```sh
make build
python3 tools/audit_speech_rates.py \
  target/release/omnivox \
  --target espeak \
  --rates 0,0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1,1.2,1.5,2 \
  --repetitions 3 \
  --json-output target/rate-audit-espeak.json
```

Use an exact `ENGINE=VOICE` target when comparing a calibration voice. Piper
also requires `--piper-model`. Under WSL, pass the Emacsvox Windows launcher,
use `--windows-output-paths`, and put `--work-dir` on a Windows-mounted path.

The JSON report records the corpus hash, executable hash and version, raw and
canonical durations, and median words per minute. It deliberately does not
record corpus text or an absolute executable path.

## Interpreting equal rates

Equal host rates are approximate at the reference voices, not an acoustic
guarantee for every voice and language. Voice models pronounce the same text
differently, and words per minute is only a practical proxy for perceived
speed.

Native limits also remain real. A table saturates when an engine cannot reach
the reference speed. In particular, eSpeak, RHVoice, and Piper exhaust their
measured headroom earlier than Eloquence. Rate remains monotonic at and above
that point, but further host increases cannot make that engine faster.

See [ADR 0004](adr/0004-per-engine-speech-rate-calibration.md) for the policy
and rationale.
