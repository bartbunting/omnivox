# Development shortcut with the normal Emacsvox profile

The Development shortcut now uses the qualified early-speech runtime with the
normal `/home/bart/.emacsvox.d` profile. Its `early-init.el` and `init.el` load
the existing literate configuration at
`/home/bart/emacs/site-lisp/emacsvox-config.org` and the profile's `custom.el`.
The launcher selects Emacsvox `0c72147cc` and the previously qualified Windows
Omnivox runtime `4e1c1561f51908a6`. No personal configuration file was edited.

This follows positive listening feedback on the
[isolated early-speech prototype](2026-11-10-early-speech-refresh.md).
The normal profile now supplies packages, key bindings, voices, speech rate,
themes, resources and notification settings. The launcher sets the real home
and XDG directories, enables the native guard and early speech, and retains
WSL resource translation and the qualified private Emacs native-code cache.
It does not force the isolated prototype's two-stream setting or test buffer.

The speech engine configuration remains separate from an installed release. It
is now persistent under the Windows local application directory at
`Emacsvox/Omnivox/profiles/development-lightweight-reload`. Its initial configuration
retains the qualified engine setup, with macOS, Piper and MBROLA disabled for
this Windows guarded prototype. Later edits are preserved. Eloquence is now the
launcher startup preference; the personal profile's saved voice choices load
normally. This does not add engine qualification or change native ownership.

## Verification

- Executable hash and compiled-client preflight pass.
- Launch inspection verifies the real profile, client checkout, early-speech flag
  and early-init → guard bootstrap → personal init ordering.
- A fresh batch Emacs completes guarded Flite source speech using the persistent
  development engine configuration and then retires its owned processes.
- The actual Windows `evox.bat` entry point passes `--check-launch` and resolves
  to the new launcher and personal profile.

The automated speech check uses disposable Emacs resources and null output;
it does not load personal integrations or start a personal Emacs server.
The complete personal profile will be exercised on the next interactive launch.
Existing Emacs sessions were not restarted. `--batch-check` and `--gui-check`
continue to use the previously qualified disposable test profile, keeping
recovery-test changes away from personal settings.

The [deployment record](data/2026-11-11-personal-development-launch/deployment.json)
identifies the wrapper backup and desktop-check log. The
[launcher](data/2026-11-11-personal-development-launch/start.py),
[bootstrap](data/2026-11-11-personal-development-launch/bootstrap.el),
[speech check](data/2026-11-11-personal-development-launch/check.el) and their
outputs are retained beside it. The existing Windows shortcut and batch wrapper
remain in place; the Linux wrapper now selects this launcher. This is a local
Development handoff, not a release or an installed-runtime replacement.

## Eloquence startup follow-up

The launcher default now selects Eloquence before the personal profile loads,
matching its current first-choice engine. An explicit `--engine` option still
selects another startup engine. The Windows shortcut's `--check-launch` reports
Eloquence, and a fresh guarded worker completes Eloquence source speech with null
output. The [startup result](data/2026-11-11-personal-development-launch/eloquence-startup-check.log)
and [desktop check](data/2026-11-11-personal-development-launch/eloquence-desktop-check.log)
are retained separately from the original Flite qualification. No running user
session or saved personal voice preferences were changed.
