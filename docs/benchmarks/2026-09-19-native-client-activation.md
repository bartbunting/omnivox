# Native parameter client activation

The complete native bundle is now advertised: catalogue discovery, engine voice
parameters, timeline 5 and playback marker 4. Engine-specific qualification and
strict preview validation remain required. No runtime or dependency is added.

## Verification

Full Emacsvox Windows development staging passed as `6e55a493cb9d2562`, based
on Omnivox `1ffca5a` with the capability advertisement and test/documentation
changes. Production executable sources did not change after this staging.
The package retains the prior development profile's absence of Piper.

The fresh-Emacs opt-in `test/verify-omnivox-native-runtime.el` in Emacsvox
passed against that package with installed DECtalk and Eloquence runtimes.
It exercised current compiled client code, real asynchronous catalogue reads,
editor changes, original/edited comparisons, and native engine readback for
DECtalk smoothness and Eloquence breathiness. Temporary palette files preserved
native settings across save/reload. Independent main and notification workers
acknowledged registration and ordinary native speech; replacement of the main
worker also passed fresh negotiation, metadata discovery and re-registration.
The harness uses null audio and private workers, never personal palette writes.
Explicit startup-busy catalogue replies are retried within a bounded harness
deadline; the production view still requires explicit Refresh.

The locked workspace passed 1012 distinct tests plus one child-process rerun,
with two expected skips. Workspace Clippy, formatting, documentation links and
whitespace checks passed. Emacsvox's current byte-code and documentation checks
also passed. Earlier qualified helper cancellation, reset, fallback, streaming
and explanation evidence remains applicable; this slice changes negotiation.

The development launcher selects the new content-addressed runtime. A fresh
development Emacs loaded the compiled engine-controls editor and negotiated
the native bundle and version-3 registrations on both streams. The previous
launcher is retained for rollback and existing Emacs sessions remain open.

Null-output acceptance establishes engine execution and consumed-source
evidence, not acoustic quality. Listening through the fresh development profile
remains user acceptance; no tagged release or general installation was changed.
