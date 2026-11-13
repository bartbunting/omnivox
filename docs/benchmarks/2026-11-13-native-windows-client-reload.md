# Native Windows Emacs: preference reload and guarded recovery

This is functional acceptance for a local development deployment, not a release
or latency measurement. Runs began on 2026-11-12 UTC
(2026-11-13 in Sydney). Native Windows Emacs 31.1 uses Emacsvox `0c72147cc`
with the [client patch](data/2026-11-13-native-windows-client-reload/client.patch).
The unchanged Windows runtime is build `4e1c1561f51908a6`, already qualified by
the [Windows multi-engine work](2026-11-10-windows-multi-engine.md) and subsequent
[early-speech follow-up](2026-11-10-early-speech-refresh.md).

## Change and scope

Local ownership negotiation previously reused the predicate for the bundled
POSIX engine-file settings provider. Native Windows therefore launched ordinary
workers without the local ownership extension, leaving host preference reload
unavailable. The client now also recognizes an absolute native Windows
`omnivox.exe` path and negotiates its existing help-advertised owner capability.
Older executables retain ordinary startup. Remote, UNC and unrelated targets
remain excluded. Native settings-service requests use the selected executable;
POSIX engine-file settings and environment translation remain separate.

The personal launcher enables the existing native guard. Saved punctuation,
capital cues, chunking and Reset defaults use `M-x omnivox-reload-preferences`
without replacing either worker. Engine, routing and output changes still need
their explicit startup/activation path. Retained recovery and minimal emergency
speech retain the established native cleanup requirements; no PID recovery or
replacement of missing retirement evidence was added.

The existing runtime and matching data were copied without rebuilding them.
All 812 copied runtime/data files match their previous hashes. The data now sits
beside `omnivox.exe`, including `espeak-ng-data/phontab`, so minimal emergency
startup can find it without ordinary saved preferences or inherited environment.
The executable SHA256 remains
`529499252023e106b0329e670b13f85fb9cb3a6b729acddabe394b92208dee28`.
See the [runtime manifest](data/2026-11-13-native-windows-client-reload/runtime-files.json)
and [staging script](data/2026-11-13-native-windows-client-reload/stage-runtime.ps1).

## Native checks

The [native fixture](data/2026-11-13-native-windows-client-reload/run-native-windows-reload.el)
runs fresh compiled Windows clients with disposable configuration, voice and
state directories, using null audio output. Each engine passes all seven stages:

1. Main and notification lanes attach to real native guards over Windows pipes
   and complete speech using the requested engine.
2. A saved chunking change applies on both existing workers, increasing live
   policy revision without changing startup identity or incarnation.
3. Malformed saved settings are rejected while speech continues; retained
   recovery restores main speech without replacing notifications.
4. Closing the client control bridge reconnects to the same guard authority.
5. Killing the exact test-owned main process still permits guarded cleanup and
   emergency eSpeak, with notification speech continuing on the selected engine.
6. Explicit restart returns main speech to the saved selected engine.
7. Main reload applies and speech continues when notifications are unavailable.

| Engine | Accepted native run |
| --- | --- |
| WinRT | [16:18:28 UTC](data/2026-11-13-native-windows-client-reload/runs/native-winrt-20261112T161828Z/result.json) |
| Flite | [16:19:00 UTC](data/2026-11-13-native-windows-client-reload/runs/native-flite-20261112T161900Z/result.json) |
| Eloquence | [16:21:27 UTC](data/2026-11-13-native-windows-client-reload/runs/native-eloquence-20261112T162127Z/result.json) |
| DECtalk | [16:21:39 UTC](data/2026-11-13-native-windows-client-reload/runs/native-dectalk-20261112T162139Z/result.json) |

The full graphical personal profile also passes its existing configuration,
bindings and sound-pack checks, both guarded speech lanes and unchanged host
preference reload with [WinRT](data/2026-11-13-native-windows-client-reload/native-personal-winrt.json),
[Eloquence](data/2026-11-13-native-windows-client-reload/native-personal-eloquence.json)
and [DECtalk](data/2026-11-13-native-windows-client-reload/native-personal-dectalk.json).
Both lanes complete speech using the requested engine. Personal settings are not edited by these checks. Native
byte-code build and preflight pass for 377 modules. The focused source suite
has 707 expected results, no unexpected results and nine native/graphical fixture
skips; the native runs above supply separate integration evidence. Linux
byte-code, header and documentation checks also pass. Rust code and payloads
did not change, so no Rust rebuild or repeat native runtime qualification was
needed for this client correction.

## Failed observations and remaining scope

The first [native run](data/2026-11-13-native-windows-client-reload/runs/native-winrt-20261112T161005Z/result.json)
passed reload and retained recovery but could not prepare emergency speech.
That exposed the previous split eSpeak data layout; the adjacent layout above
corrects it without weakening emergency isolation.

Additional early-speech runs exposed intermittent timeline rejection while
optional discovery replaced the voice registry. The
[captured diagnostics](data/2026-11-13-native-windows-client-reload/runs/native-winrt-20261112T161736Z/diagnostics.txt)
report `timeline registry generation is not current` after a restart and reload.
The existing native shortcut already disabled early speech; the accepted runs
and deployment preserved that eager-startup setting. The separate discovery
admission race remained unresolved at this checkpoint. The
[subsequent correction](2026-11-13-native-early-start.md) records its resolution
before enabling the opt-in in the native profile. No claim is made that all other
platforms were unaffected.

One [intermediate test](data/2026-11-13-native-windows-client-reload/runs/native-winrt-20261112T161432Z/result.json)
also assumed callback completion order matched lane order. The fixture now
checks results by `Main` and `Notifications`, consistent with independent
installation. The first four WinRT runs used early speech; later accepted runs
used eager startup. Failed logs remain retained alongside passing results.

After deployment, Bart confirmed on 2026-11-13 that speech is working. This
provides general audible-speech acceptance for the updated native shortcut;
individual engines and reload/recovery listening scenarios were not itemised.
The automated checks do not establish acoustic timing, device-loss recovery,
performance equivalence, all-engine native-client coverage or owner-and-guard
combined-loss recovery. Existing Windows runtime qualification and the
[Development shortcut daily-use confirmation](2026-11-11-personal-development-launch.md#daily-use-confirmation-2026-11-13)
remain separate evidence.

## Deployment and reproduction

The tested build is installed at
`%LOCALAPPDATA%\Emacsvox\NativeLightweight\native-reload-20261113`.
It retains native Windows Emacs and the existing personal profile, configuration
and voice roots. The desktop `Emacsvox Windows` shortcut now selects its launcher;
readback confirms the target and working directory, with arguments and icon
preserved. The [deployment record](data/2026-11-13-native-windows-client-reload/deployment.json)
records the previous target, backup, checked payload hashes and zero remaining
candidate runtime processes after the tests. The original shortcut and runtime
remain available for rollback.
One fresh launch is required to load the changed client; subsequent supported
host-preference changes use reload. Existing running sessions are not changed.

The [runner](data/2026-11-13-native-windows-client-reload/run-native.ps1)
accepts `-Engine winrt`, `flite`, `eloquence` or `dectalk`. It requires the staged
client/runtime and local native installation metadata. `-EarlySpeech` reproduces
the separate experimental mode, which was not the default at this checkpoint.
The subsequent [early-start correction](2026-11-13-native-early-start.md)
records the generation fix and the native launcher's Eloquence default.
The [personal launcher](data/2026-11-13-native-windows-client-reload/evox-windows.ps1)
supports `-Check` for a disposable graphical process using the personal profile.
The [provenance](data/2026-11-13-native-windows-client-reload/provenance.json)
records source, patch, payload and check identities. Private owner credentials,
mutable native management state and licensed engine data are excluded.
The [checksums](data/2026-11-13-native-windows-client-reload/SHA256SUMS) cover the
retained evidence without changing native log line endings or source snapshots.
