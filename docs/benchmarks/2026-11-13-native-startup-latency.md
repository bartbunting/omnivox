# Native desktop startup: remove repeated build audits

The personal Windows launcher now starts one Emacs process on ordinary launches.
Its previous version started two additional batch Emacs processes to validate
the Emacs version and the complete client byte-code inventory. Those audits
remain available through `-Check` and deployment preparation. The personal
bootstrap checks its running native Windows Emacs version before loading the
profile. This personal-launcher change is workstation-config commit `c8b0653`;
the client and speech runtime are unchanged.

## Measurements

Two alternating before/after pairs used the same C# executable as the desktop
shortcut, Windows Emacs 31.1, the personal profile, default Eloquence and guarded
early speech. Timing begins immediately before Windows `Process.Start` of that
launcher and ends at the first client-observed `utterance_started` event. It
includes PowerShell startup, validation, package loading and speech startup.
It excludes Explorer's shortcut dispatch and physical acoustic output.

| Launch | First speech source | Personal profile loaded |
| --- | ---: | ---: |
| Before, pair 1 | 11.993 s | 14.049 s |
| After, pair 1 | 8.449 s | 10.524 s |
| Before, pair 2 | 12.686 s | 15.064 s |
| After, pair 2 | 8.080 s | 10.090 s |
| After, DECtalk check | 8.730 s | 10.690 s |

The median first-source difference is about 4.1 seconds, or 33%. These are
warm-cache local screening samples, not cold-start or tail-latency guarantees.
Identical load-event and milestone instrumentation was used in both arms.
The [summary](data/2026-11-13-native-startup-latency/summary.json), raw timing
events and [runner](data/2026-11-13-native-startup-latency/run-native-wrapper-timing.py)
are retained. Earlier script-only samples also found approximately four seconds
of savings; they exclude C# and PowerShell initialization.

In one script-only baseline the version probe took 0.89 seconds and the
byte-code check 2.64 seconds. Package initialization in the wrapper comparisons
took 2.61–3.74 seconds. Package loading remains unchanged. A quickstart cache
was generated only in the disposable investigation directory; further package
experiments were cancelled before any cache was enabled or measured.

## Verification and deployment

Fresh native graphical checks pass the complete personal configuration,
Eloquence and DECtalk on both speech lanes, unchanged preference reload and
owned process retirement. Explicit `-Check` still completes the native version
and byte-code audits before the graphical check. PowerShell syntax and Lisp
parentheses checks pass. No Rust or Emacsvox Lisp implementation changed, so
the existing compiled client and runtime were reused.

The [deployment record](data/2026-11-13-native-startup-latency/deployment.json)
contains original backups and before/after launcher hashes. Desktop
[readback](data/2026-11-13-native-startup-latency/readback.json) verifies the
shortcut still targets the qualified stage, the deployed scripts parse, and
no candidate runtime processes remain after testing. The maintained launcher,
desktop stage script and personal `bin` script receive the same change.
Shared startup configuration and saved speech preferences are unchanged.

The client remains `0acf2deae`, with runtime build `4e1c1561f51908a6` and
executable SHA256
`529499252023e106b0329e670b13f85fb9cb3a6b729acddabe394b92208dee28`.
The [patch](data/2026-11-13-native-startup-latency/launcher.patch),
[provenance](data/2026-11-13-native-startup-latency/provenance.json) and
[checksums](data/2026-11-13-native-startup-latency/SHA256SUMS) identify the tested
inputs. Full private speech logs, authentication files and licensed payloads
are excluded.

The first timing attempt completed its functional check and retired its speech
processes, but its timing exporter supplied a Lisp list instead of a JSON array.
The export error prevented normal Emacs exit; the runner timed out and killed
its own test Emacs. That attempt and a batch tracer check without the client
environment are retained separately and excluded from timing claims.
