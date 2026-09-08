# Windows native voice default audit

All 17 advertised Windows voices passed 85 captured syntheses: eight Eloquence
voices and nine DECtalk voices, each tested in the sequence default, set,
default, set, default. No audio device was opened. No installed runtime or live
Emacsvox configuration was changed.

The tested helpers came from Omnivox 1.9.0 source
`69d1e9496be68eaa27f06908b10b2fc5591a9918`, Emacsvox build
`68a18510726c9676`. Their Windows helper source is unchanged in routing slice
`97481e7`. [Captured results](data/2026-09-09-windows-native-defaults.txt)
include SHA-256 hashes of both helper executables and both native DLLs.

## Method and result

[The audit](../../tools/check_windows_voice_defaults.ps1) loads the existing
helper capture classes and the adapters' actual extended-ACSS mapping methods
by reflection. All ECI construction, synthesis, parameter queries and disposal
run on one x86 STA thread. PCM is captured into memory and checked non-empty.
The test queries native values after synchronous synthesis finishes.

Eloquence uses `eciGetVoiceParam` for all eight active-voice parameters:
gender, head size, pitch baseline, pitch fluctuation, roughness, breathiness,
speed and volume. This API is documented in the
[IBM ViaVoice Outloud programmer's guide](https://www.cs.columbia.edu/~hgs/research/projects/simvoice/simvoice/docs/tts.pdf).
DECtalk uses `TextToSpeechGetSpeakerParams` for smoothness, assertiveness,
average pitch, pitch range, richness, baseline fall, quickness, hat rise and
stress rise. Its returned parameter buffers are freed with `FreeCoTaskMem`.

For each voice, customized pitch and extended parameters change the queried
values. Each later request with omitted extended fields and the voice's usual
pitch restores exactly the original native values. A second customized request
reproduces its first customized values. Consequently, the existing voice
selection prefixes establish fresh native defaults for the tested runtimes;
no new reset command, helper protocol or calibration change is needed.

PCM hashes are deliberately not the oracle: repeated untuned utterances can
produce different bytes or initial buffer lengths. That variation was observed
before selecting native parameter queries for this test.

## Repeat

Run from 32-bit Windows PowerShell with installed, licensed x86 runtimes:

```powershell
& "$env:WINDIR\SysWOW64\WindowsPowerShell\v1.0\powershell.exe" `
  -NoProfile -NonInteractive -Sta -File tools\check_windows_voice_defaults.ps1 `
  -Helpers C:\path\to\matching\helpers `
  -EciDll C:\path\to\ECI.DLL `
  -DectalkDll C:\path\to\DECtalk.dll
```

The audit fails if query exports are absent, native synthesis is empty, tuning
has no effect, repeated tuning differs, or a default request fails to restore
the baseline. It is an optional installed-runtime audit, not a substitute for
the portable fake-engine routing tests.

Limits: this tests the Windows capture/native boundary and its actual mapping
methods. It does not yet test full layered admission, progressive playback
observation, post-synthesis effects or Linux runtime versions. Native rate and
volume are held independently of extended defaults; this does not redefine
host-rate calibration. Full helper-protocol and playback acceptance remain in
the [implementation sequence](../per-fallback-streaming-handoff.md).
