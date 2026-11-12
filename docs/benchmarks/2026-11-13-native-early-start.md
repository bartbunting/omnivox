# Native early startup: stable speech registration

This functional follow-up corrects the client-side discovery race found during
[native Windows reload qualification](2026-11-13-native-windows-client-reload.md).
It also aligns the personal native launcher's default engine with Development.
These are local development changes, not a release or a latency claim. Tests
ran on 2026-11-12 UTC, 2026-11-13 in Sydney.
The final Emacsvox implementation is `0acf2deae`, following the pushed native
reload commit `a366a7fe9`; the maintained launcher default is workstation-config
`80cd555`. The [client patch](data/2026-11-13-native-early-start/client.patch)
and [provenance](data/2026-11-13-native-early-start/provenance.json) identify the
tested source and payload separately from publication.

## Cause and correction

The server installs a registration before Emacs observes its acknowledgement.
Automatic discovery previously allocated a new generation even when that lane's
native voice definitions were unchanged. Speech submitted during the reply gap
still named the old generation and was correctly rejected by the server.

The client now reuses the lane's acknowledged native registration generation
when its complete definitions and client provenance match. Re-registering still
refreshes availability and native status. Genuine definition changes retain
their normal new-generation path. The client records the highest registration
generation sent, even through timeout, write failure or a delayed older request.
A newer unconfirmed generation blocks reuse of an older acknowledgement.
Process identity, server admission, explicit configuration-apply semantics and
native ownership remain unchanged. No Rust source or protocol was changed.

The [source regression](data/2026-11-13-native-early-start/regression-before.log)
fails against the previous client with speech generation 2 and server generation
3. A [native baseline](data/2026-11-13-native-early-start/runs/native-winrt-20261112T164130Z/result.json)
also fails when the fixture deliberately withholds the registration reply after
the real server has installed it. The corrected client speaks successfully
before that reply is released. The second
[regression](data/2026-11-13-native-early-start/highest-generation-before.log)
rejects an intermediate implementation that forgot a newer write when a delayed
older request was sent.

## Verification

The final source suite passes 709 tests, with no unexpected results and nine
native/graphical fixture skips. A separate targeted selection passes 53 control
lifetime and queue tests. Linux byte-code, native Windows byte-code build and
preflight for 377 modules, headers and documentation checks pass. Native checks
use Windows Emacs 31.1 and the unchanged qualified runtime
`4e1c1561f51908a6`; its executable SHA256 remains
`529499252023e106b0329e670b13f85fb9cb3a6b729acddabe394b92208dee28`.

Each final native run checks speech during a deliberately delayed registration
reply, both guarded lanes, in-place host preference reload, rejection of damaged
settings, retained recovery, same-guard reconnection, owner-loss emergency eSpeak,
return to the saved engine and foreground reload with notifications absent.

| Engine | Final native result |
| --- | --- |
| WinRT | [16:57:05 UTC](data/2026-11-13-native-early-start/runs/native-winrt-20261112T165705Z/result.json) |
| Flite | [16:57:17 UTC](data/2026-11-13-native-early-start/runs/native-flite-20261112T165717Z/result.json) |
| Eloquence | [16:57:28 UTC](data/2026-11-13-native-early-start/runs/native-eloquence-20261112T165728Z/result.json) |
| DECtalk | [16:57:38 UTC](data/2026-11-13-native-early-start/runs/native-dectalk-20261112T165738Z/result.json) |

The full graphical Windows profile also passes both lanes and unchanged reload
with [default Eloquence](data/2026-11-13-native-early-start/native-personal-default.json),
[DECtalk](data/2026-11-13-native-early-start/native-personal-dectalk.json) and
[WinRT](data/2026-11-13-native-early-start/native-personal-winrt.json), all with
early speech enabled. Compiled WSL clients pass reload, restart, retained
recovery, emergency speech and return to saved settings with
[Eloquence](data/2026-11-13-native-early-start/wsl-eloquence.log) and
[DECtalk](data/2026-11-13-native-early-start/wsl-dectalk.log).

These checks observe source completion, not physical acoustic timing. The
earlier native audible-speech confirmation belongs to the eager-start build;
listening acceptance of this early-start correction remains separate.

## Profile checks and intermediate observations

An [immediate personal-profile probe](data/2026-11-13-native-early-start/personal-failure-1/native-guarded-personal-check.json)
failed before observing completed speech. Tracing showed that saved presentation
restoration changes the definitions after the first startup registration and
applies them asynchronously. A delayed older notification request can also arrive
after a newer startup registration; the highest-generation record handles that
ordering without forgetting the newer request.

The final profile probe waits for acknowledgement of the actual saved startup
definitions, rather than accepting any earlier startup snapshot. This does not
weaken the discovery regression: its independent fixture deliberately speaks
while an unchanged registration's reply is withheld. Diagnostic-message speech
is suppressed during the probe and teardown. The initial failed profile process
timed out after recording failure; a subsequent tracing attempt had a fixture
parenthesis error. Those attempts are retained and are not passing evidence.
Seven intermediate isolated engine runs passed before the final highest-generation
correction; the table above identifies the four final runs.

## Shared configuration and deployment

The [configuration comparison](data/2026-11-13-native-early-start/shared-config.json)
confirms that `early-init.el`, `init.el` and `emacsvox-config.org` are identical
between WSL and native Windows. The different startup voice came from the native
launcher's explicit WinRT default. It now selects Eloquence by default, matching
Development, while preserving an explicit `OMNIVOX_ENGINE` override. No personal
voice settings, package snapshots or platform-specific paths were copied over.
The [eager-default check](data/2026-11-13-native-early-start/eager-default-eloquence.json)
verified this correction before enabling early startup.

The desktop `Emacsvox Windows` shortcut now selects the verified build under
`%LOCALAPPDATA%\Emacsvox\NativeLightweight\native-early-20261113` with the
existing personal profile, engine configuration and voice roots. Early startup
is enabled and Eloquence is the default. The
[deployment record](data/2026-11-13-native-early-start/deployment.json) confirms
shortcut readback, the backup of the previous target, verified payload hashes
and zero remaining candidate runtime processes after the tests. Existing running
sessions remain unchanged; a fresh launch loads the correction. The retained
[runner](data/2026-11-13-native-early-start/run-native.ps1) accepts `-EarlySpeech`
and `-Engine`; the [fixture](data/2026-11-13-native-early-start/run-native-windows-reload.el)
requires an explicitly isolated test directory. The
[personal check](data/2026-11-13-native-early-start/check-personal.ps1) exercises
the native launcher and existing configuration. Runtime credentials, mutable
ownership records, private configuration contents and licensed payloads are
excluded from the retained evidence.
The [checksum manifest](data/2026-11-13-native-early-start/SHA256SUMS) preserves
the raw reports, failed attempts and reproduction inputs.
