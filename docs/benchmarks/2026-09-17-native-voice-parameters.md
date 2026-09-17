# Windows native voice parameter audit, 2026-09-17

The installed Eloquence 6.1.0.0 and DECtalk v4.99 GitHub NORMAL ACCESS32
runtimes passed **316 individual parameter cases across 17 voices**, using
649 silent, nonempty PCM captures. This is interior-value and ordinary reset
evidence, not complete range/cancellation qualification or an audible test.

- Eloquence: eight controls on each of eight voices; 136 captures.
- DECtalk: all 28 documented design-voice controls on each of nine voices;
  513 captures.
- All tested writes read back the requested native value.
- The next ordinary voice-selection request restored every audited field.
- For Eloquence, all 64 direct `eciSetVoiceParam` writes returned the previous
  value and read back correctly; `eciCopyVoice(preset, 0)` restored all eight
  pristine preset values after each write.

No installed files, live worker, Emacs session or audio output device changed.
The audit loaded existing capture classes from development payload
`e1ecdb481ee08fd0` in separate x86 STA PowerShell processes. Each process had
a 60-second outer timeout. PCM stayed in capture memory and was discarded.

## Reproducible inputs and evidence

The [audit script](../../tools/check_windows_voice_parameters.ps1) compiles the
[test-only probe](../../tools/NativeVoiceParametersAudit.cs), loads the selected
helper's capture class, and queries the exact DLL already loaded by the helper's
validated native loader. The probe uses Cdecl for DECtalk and StdCall for ECI.
DECtalk's four successful query buffers are freed with `FreeCoTaskMem`.

The [Eloquence results](data/2026-09-17-eloquence-parameters.json) and
[DECtalk results](data/2026-09-17-dectalk-parameters.json) contain every case,
runtime/helper/probe hashes, native version, tested value, observed value,
changed field IDs, baseline, reported default and reset result. No DLL or
dictionary is distributed with this evidence.

DECtalk runtime SHA-256:
`ac0bda78b1c42f2503eb9ce1b85096b3555099cf2282a941cf73cc80f2370583`.
ECI runtime SHA-256:
`da99080288cdca14a7effba20274af1d6d5878840e32be5a315bd8691124703b`.

Run one engine at a time from Windows, with an external process timeout:

```powershell
& "$env:WINDIR\SysWOW64\WindowsPowerShell\v1.0\powershell.exe" `
  -NoProfile -NonInteractive -Sta -File tools\check_windows_voice_parameters.ps1 `
  -Engine eloquence -Helper C:\path\OmnivoxEloquenceHelper32.exe `
  -RuntimeDll C:\path\ECI.DLL
```

For DECtalk use `-Engine dectalk`, its matching helper and DLL/dictionary.
The script emits JSON and exits unsuccessfully if any candidate fails.
A locally reviewed script on a UNC path may require process-scoped execution
policy when local policy permits it; this does not require changing machine or
user execution policy. Run in a fresh process, not an existing speech worker.

## What this establishes

The probe selects each voice, captures a baseline utterance, changes one
control by approximately one tenth of its range, reads all audited native
fields, then captures an ordinary utterance and compares all fields with the
baseline. ECI controls also receive independent setter/copy checks. Each tested
control changed only its own queried field; that does not prove acoustic
independence or absence of unqueried internal interactions.

DECtalk supplies limits and defaults through `TextToSpeechGetSpeakerParams`.
The probe reads only the 28 initialized documented fields, not unused or
reserved SPDEFS slots. All 28 controls, including the nine gain controls,
worked on this particular build. Its reported `ap` range is 50–350 Hz; the
existing common adapter clamp is 50–500. Native-edit limits must follow the
qualified runtime profile, while preserving the existing common mapping.
Do not replace common calibration or generalize this build's limits to every
DECtalk release. The local source implementation also contains uninitialized
or inconsistently assigned additional SPDEFS slots, so bulk struct serialization
is unsuitable for a public descriptor.

ECI limits in this probe come from the documented ECI-units profile, not a
runtime range-query API. Defaults come from the copied preset through native
readback. The source-backed DECtalk API layout and the earlier
[common-default audit](2026-09-09-windows-native-defaults.md) provide the audit
baseline; no arbitrary native commands are exposed to end users.

## Qualification still required before advertising writable controls

- Valid range boundaries, clamping, invalid inputs and runtime/profile changes.
- Cancellation and failure between native application and PCM commitment.
- Actual default operations and typed dependency plans through helper protocol 6.
- Multiple styled spans and ordinary/preview interleaving on the same instance.
- ECI unit-mode verification, dialect variation and optional export absence.
- Native volume ownership on each platform. Windows currently combines volume
  and richness in the native ECI setting; Linux applies requested volume in
  `linux-helpers/src/native.rs` after its native richness compensation.
- Linux native runtime acceptance, and other platforms before claiming support.

The existing helpers still advertise their old protocol and capabilities.
This audit is qualification infrastructure, not a native-control UI or runtime
implementation. New codecs and composition must consume the independent
contract fixtures before the complete feature bundle is advertised.
