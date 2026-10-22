# Punctuation configuration in two stages

Status: both implementation stages are complete; remaining listening and native
editor acceptance keep this plan active. Reviewed 2026-10-23. Built-in table
configuration shipped in 1.15.0 and named profiles in 1.17.0. A fourth built-in
`most` level remains a separate, unimplemented scope.
Scope: host punctuation tables, named profiles and paired Emacsvox selection.
Related: [roadmap](../ROADMAP.md), [configuration contract](../reference/engine-configuration.md).

Omnivox defines character pronunciations and applies them before engine selection.
Emacsvox selects the active level for the current mode or buffer. This follows
the host configuration and immutable startup boundaries in
[ADR 0008](../adr/0008-extensible-engine-registration.md) and
[ADR 0009](../adr/0009-local-speech-preferences.md).

The motivating report concerns apostrophes in Org prose. Org selects `some` by
default; the previous server left both straight and curly apostrophes untouched
there. At `all`, it expanded only the straight apostrophe. Source extraction
retained both characters. This identifies a server table gap, but does not
establish what the reporter's particular voice pronounced.

## Stage one: configure the existing levels

Status: implemented for 1.15.0. The [candidate report](../benchmarks/2026-09-28-1.15-candidate.md)
records Linux/Windows configuration and editor checks and remaining listening/platform limits.

Scope: implement sparse pronunciation overrides for `none`, `some` and `all`
in the speech host's existing `config.json`. Preserve ASCII defaults and add
common Unicode punctuation, including curly apostrophes at `all`. A spoken name
and preservation of the original character are distinct choices. Omitted entries
inherit shipped defaults; levels are independently configurable. Do not delete
unannounced punctuation, which engines may need for pronunciation and pauses.

The [configuration reference](../reference/engine-configuration.md#punctuation-tables)
specifies the format, validation, defaults and compatibility. The
[configuration guide](../guides/configuration.md#punctuation-pronunciations)
explains setup. Capture complete resolved tables for both workers, admitted
requests, reset and recovery; loading edits requires deliberate restart or Apply.
No helper or speech-protocol change, Emacsvox level-selection change, automatic
reload or live deployment is part of this stage.

The review/edit interface provides `M-x omnivox-punctuation` in Emacsvox and a negotiated
[local editing service](../reference/engine-configuration.md#local-punctuation-editor)
in Omnivox. The host supplies defaults, validates and atomically saves only the
punctuation overrides, and checks the reviewed file revision. Emacsvox provides
spoken character rows, independent drafts, preservation/default actions, and
separate Save and Restart commands. Saving alone never activates settings.
Stage two extends this editor with named profile drafts;
built-in-only saves preserve existing profiles.

Acceptance:

- Every existing ASCII pronunciation remains unchanged with no override.
- Straight, curly and modifier apostrophes are preserved at `some` and named
  at `all` without a file; a user can opt into names at `some`.
- Custom names, explicit preservation, unknown Unicode, the legacy `[*]`
  separator and original UTF-8 action offsets work in both preparation paths.
- Malformed, duplicate, unknown-level and oversized configuration fails before
  engine construction. Historical startup records retain their original shape
  and ASCII behavior.
- Actual helper requests confirm that both workers and retained recovery use
  the captured tables even after files change. Character inspection remains
  independent of prose punctuation selection.

<a id="stage-two-custom-named-profiles-deferred"></a>

## Stage two: custom named profiles (implemented in 1.17.0)

A fourth built-in level, `most`, remains deferred. Its exact
table and client/server negotiation need a separate implementation scope;
adding it must preserve existing levels and define older-server fallback.
The 1.15.0 defaults name apostrophes only at `all`.

Users can define profiles such as `prose`, `code` or `proofreading` by inheriting
one resolved built-in level and supplying sparse character overrides. Inheritance
is restricted to built-ins; `none`, `some` and `all` retain stable meanings and
reserved IDs. Profiles share the immutable startup record with built-in tables.

The implemented `punctuation_profiles_v1` capability provides these semantics:

- Omnivox advertises available stable profile IDs and their built-in fallback.
- Emacsvox discovers them from both workers, provides selection, and allows
  mode/buffer preferences without containing pronunciation tables itself.
- An old server receives the chosen built-in fallback. A server supporting
  profiles rejects an unknown ID explicitly rather than silently substituting.
- Missing, unsupported or disagreeing worker catalogues select the saved built-in
  fallback and expose that state; named selection requires agreeing descriptors.
- Isolated character inspection remains independent of the chosen prose profile.

The implementation is specified in the
[version-4 configuration contract](../reference/engine-configuration.md#configuration-version-4-named-punctuation-profiles).
Emacsvox creates and edits named drafts, discovers worker catalogues, and retains
built-in fallbacks in mode/buffer selections. `verify_engine_configuration.py`
checks actual helper text, shared startup/recovery and unknown-profile rejection.
Audible acceptance remains separate from automated preparation checks.
Acceptance must cover negotiation, old clients and servers, missing profiles,
both workers, saved preferences, reset, recovery, and source-offset parity.
Custom names are not sent through the three-value punctuation enum.

## Remaining acceptance and follow-up

- Complete native macOS editor and listening acceptance; Linux/Windows automated
  configuration and recovery checks do not establish audible pronunciation.
- Extend live-client checks for both workers, saved mode/buffer selections,
  old-peer fallback, missing profiles, reset/recovery and source-offset parity.
  Preserve the existing verification results rather than treating this checklist
  as evidence that those scenarios have no automated coverage.
- Specify the `most` table, negotiated support and old-server fallback separately.
  It is not supplied by named profiles and is not needed to complete their
  implementation.
