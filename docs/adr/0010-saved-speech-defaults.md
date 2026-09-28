# ADR 0010: Saved speech defaults and reset

- Status: Accepted
- Accepted: 2026-09-28; the maintainer chose saved settings, rather than built-in
  settings, as the speech-reset baseline.
- Extends: [Local speech preferences](0009-local-speech-preferences.md).
- Related: [Engine registration](0008-extensible-engine-registration.md),
  [voice tuning](0006-voice-selection-and-customization.md),
  [worker ownership](0004-workstation-service-and-worker-ownership.md).

## Context

Startup voice and speech controls currently require launcher arguments or client
commands. A host configuration should provide a useful baseline while allowing
Emacs to select different settings. Resetting to unrelated built-in values would
make saved preferences unreliable.

## Decision

Extend the unreleased version-2 configuration with `speech.defaults`: optional
voice, normalized rate, pitch, speech/tone/sound gains, punctuation, CamelCase
splitting and character-rate scale. Fill omitted members with the existing
built-in values. Validate types and bounded ranges before native construction.
A saved voice is the same preference as `--voice`, not a new engine selector or
a claim that a voice is installed.

Use saved values at startup, then apply existing command-line overrides. Runtime
client commands and explicit voice styles retain their existing priority. The
existing reset command stops speech, clears pending work and transient delays,
and restores the saved speech values. Command-line speech overrides are startup
choices; reset restores the file baseline, and later client commands can override
it again. Missing configuration preserves the old built-in speech defaults.
No new command or public capability is introduced.

Freeze the complete defaults in the same activation as engine registration.
Both lanes and every recovery use those values; reset performs no file read.
Private startup schema 3 requires the complete defaults object. Historical
schemas 1 and 2 keep their original meanings and serialized shape, including
built-in speech defaults, so retained older executables can recover.

Exact speech diagnostics use the same captured defaults as engine selection,
then command-line overrides; a nonempty `--dump-wav` positional voice wins last.
Diagnostic text remains whole-text native synthesis rather than server text
preparation. Apply speech gain once in the host audio pipeline.

## Consequences and alternatives

Saved defaults become predictable without preventing temporary client choices.
This adds a bounded local configuration and private compatibility contract; it
does not add remote management, live reload, a UI, language routing, named output
devices or configurable safety limits. Process audio routing retains its existing
behavior and remains separate output-configuration work.

Resetting to built-in defaults would preserve the old implementation but defeat
the chosen saved baseline. Resetting to the latest mutable state would not reset
anything reliably. Rereading files during reset could split the two lanes or
change recovered speech unexpectedly.

The [configuration reference](../reference/engine-configuration.md#saved-speech-defaults)
defines exact fields and precedence. Tests must cover overrides followed by reset,
immutable recovery, old snapshots, malformed values, private previews and exact
diagnostics. Functional checks do not establish audible engine quality.
