# Test macOS audio-device selection

The development implementation follows [ADR 0020](../adr/0020-audio-output-device-selection.md):
main speech and notifications can use separate physical outputs, Apply preserves
speech engines, and a disconnected explicit device never falls back to speakers.
Native Intel/Apple Silicon execution and physical listening are pending. A passing
cross-target check is not a native build or listening result.

## Install the testing release

The prepared server candidate is `v1.19.0-pre.1`, paired with Emacsvox commit
[`a751af1506b73b8a37a37085eb7a356a318da8ca`](https://github.com/bartbunting/emacsvox/commit/a751af1506b73b8a37a37085eb7a356a318da8ca).
Use this client commit for the physical device menus. It is a testing checkout,
not a separately versioned Emacsvox release.

Once the prerelease and tap update are published, follow the
[prerelease installation instructions](../../.github/DEPLOYMENT.md#prereleases).
Set `OMNIVOX_PROGRAM` to `$(brew --prefix omnivox-pre)/bin/omnivox` in the isolated
session below instead of the source-build path. The source build and ignored
hardware smoke test remain optional additional checks for binary testers.

## Prepare the candidate

Use the paired reviewed Omnivox and Emacsvox candidate checkouts. This feature is
not in the pinned Omnivox 1.18.0 release. Record each checkout's `git rev-parse HEAD`
and `git status --short`, macOS version (`sw_vers`), architecture (`uname -m`),
Emacs version, and output models/connections (built-in, USB or Bluetooth).
Test on Apple Silicon and Intel separately where available.

From Omnivox, use the normal build wrapper so the executable has its eSpeak data,
companions and license notices. The repository selects Rust 1.97.1. Native source
build prerequisites are described in the [deployment guide](../../.github/DEPLOYMENT.md).

```sh
make build
target/release/omnivox --list-audio-devices > macos-devices.json
cargo test --locked -p omnivox-audio
cargo test --locked -p omnivox-audio \
  native_default_explicit_default_preserves_the_output_owner \
  -- --ignored --test-threads=1
```

The last test needs a real available output. It opens CoreAudio listeners and
streams, applies default → explicit current output → default on the same owner,
then closes them. It does not play speech or establish acoustic correctness.
The JSON should say `supported: true`, with a `macos:coreaudio:` host scope and
`coreaudio:` device IDs. Duplicate labels must remain separate choices.

From Emacsvox, select your Emacs 30.2 or newer executable through `EMACS` or
`local.mk`, then run `make bytecode-rebuild` and `make bytecode-check`. Launch the
candidate with `OMNIVOX_PROGRAM` set to the absolute path of this built Omnivox.
Use a disposable Emacs profile and Customize file for Save/restart checks, so
they do not change your everyday preferences. Keep a working reader available
while testing an intentional output outage.

For an isolated source-checkout session, adapt these two absolute paths, keeping
the same test directory when restarting:

```sh
export OMNIVOX_PROGRAM=/absolute/path/to/omnivox/target/release/omnivox
export EMACSVOX_DIR=/absolute/path/to/emacsvox
export OMNIVOX_MAC_TEST_DIR="$(mktemp -d /tmp/omnivox-mac-test.XXXXXX)"
export XDG_STATE_HOME="$OMNIVOX_MAC_TEST_DIR/state"
export XDG_CONFIG_HOME="$OMNIVOX_MAC_TEST_DIR/config"
export OMNIVOX_CONFIG_DIR="$OMNIVOX_MAC_TEST_DIR/omnivox-config"
mkdir -p "$OMNIVOX_CONFIG_DIR"
export TTS_PROGRAM=omnivox
emacs -Q --no-splash --eval '
  (progn
    (setq user-emacs-directory
          (file-name-as-directory (getenv "OMNIVOX_MAC_TEST_DIR"))
          custom-file (expand-file-name "custom.el" user-emacs-directory))
    (load custom-file t t)
    (add-to-list (quote load-path) (expand-file-name "lisp" (getenv "EMACSVOX_DIR")))
    (require (quote emacsvox-preamble))
    (setq emacsvox-user-directory (expand-file-name "resources/" user-emacs-directory))
    (make-directory emacsvox-user-directory t)
    (load (expand-file-name "lisp/emacsvox-setup.el" (getenv "EMACSVOX_DIR"))))'
```

Use your selected Emacs executable in place of `emacs` if necessary. Do not rerun
the `mktemp` assignment for the Save/restart check. If your shell supplies explicit
`OMNIVOX_AUDIO_DEVICE`, device-preference or engine-configuration overrides,
record and remove inherited overrides before running the block; explicit launch
settings take precedence. The block supplies a private empty Omnivox configuration
root because macOS does not use `XDG_CONFIG_HOME` for its engine settings. This
does not change the normal installation.

## Listening checklist

Use two independently selectable outputs, for example speakers and a USB headset.
Start with the native macOS voice, then repeat ordinary speech with bundled eSpeak.
This feature does not implement macOS Eloquence or DECtalk adapters.

1. Open `C-e H o n`. Set notification routing to Automatic, Left, Right or Both
   so notifications have their own worker. `D` opens main output devices; `M`
   opens notification output devices.
2. In either device menu, `d` opens the choices. Browse with arrows; Enter selects
   a candidate and returns without applying it. `a` applies, `g` refreshes status,
   and `s` saves the confirmed choice. Check that browsing alone leaves audio on
   its previous output.
3. Apply the headset to main and speakers to notifications. Read a buffer and use
   `N` in the routing menu to preview notifications. Reverse the devices and
   repeat. Check speech, tones and earcons, and Left/Right/Both routing on each
   stereo-capable device. Voice, rate and gains should remain unchanged.
4. Switch during a long utterance, then while idle. The interrupted utterance
   must stop without replay; fresh speech must use the selected output. Capture
   worker process IDs before and after with `M-: (mapcar (lambda (p) (and
   (processp p) (process-id p))) (list tts-speaker-process tts-notify-process))`.
   Both workers should survive device-only Apply.
5. Explicitly choose the current system default by its device entry. Change the
   macOS system output to the other device. The explicit lane must stay on its
   chosen device. Then Apply Follow system default and change the macOS output
   again: that lane should follow, cancelling rather than replaying old speech.
6. With a headset explicitly selected, unplug/disconnect it during speech and
   while idle. No speech should emerge from speakers. Refresh should retain the
   explicit selection but show it unavailable. Reconnect that same headset:
   only new speech may play. The unaffected lane should remain usable. Restore
   output with an explicit Apply if automatic reconnect fails, and report it.
7. Save different choices for the two lanes in the disposable profile, exit that
   test Emacs, and restart with the same test directory. Check both destinations.
   An Apply without Save should last for the session only. A missing saved device
   should produce an explicit startup failure, never silently use another output.
8. Repeat several switches and disconnects. If available, include Bluetooth
   profile changes and sleep/wake. Restore the macOS system output afterwards.

## Return evidence

Report each checklist item as pass, fail or not tested, the engine/voice used,
device names/connection types, and exactly what you heard. Include source
revisions, catalogue JSON, the native smoke-test output, and the test session's
logs from `$XDG_STATE_HOME/emacsvox/omnivox`. Note whether the other lane kept
working, whether worker IDs changed, and whether old audio replayed. An Apply
timeout is an unknown result; refresh it before trying another change.

Host scopes and device IDs identify the test machine and its outputs; redact
them consistently when sharing publicly. Native execution, acoustic correctness,
Bluetooth/sleep coverage and latency measurements are separate evidence.
The [protocol reference](../protocols/audio-devices.md) defines identity,
recovery and operation outcomes.
