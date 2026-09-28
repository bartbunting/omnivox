# Punctuation configuration in two stages

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
This remains an interface to the three existing levels; custom-profile
negotiation remains stage two.

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

## Stage two: custom named profiles (deferred)

A fourth built-in level, `most`, is also deferred beyond 1.15.0. Its exact
table and client/server negotiation need a separate implementation scope;
adding it must preserve existing levels and define older-server fallback.
The 1.15.0 defaults name apostrophes only at `all`.

Allow a user to define a profile such as `prose`, `code` or `proofreading` by
inheriting one built-in level and supplying sparse character overrides. Initially
restrict inheritance to built-ins so cycles and multi-level resolution are absent.
Keep `none`, `some` and `all` available with stable meanings and reserve their IDs.
Resolve profiles into the same immutable startup record as stage one.

This stage needs a separately specified capability and protocol extension:

- Omnivox advertises available stable profile IDs and their built-in fallback.
- Emacsvox discovers them from both workers, provides selection, and allows
  mode/buffer preferences without containing pronunciation tables itself.
- An old server receives the chosen built-in fallback. A server supporting
  profiles rejects an unknown ID explicitly rather than silently substituting.
- Define behavior when workers disagree or a saved profile disappears; avoid
  publishing a partially activated configuration.
- Keep isolated character inspection independent of the chosen prose profile.

The exact schema, message shapes, naming limits and UI are future design work.
Acceptance must cover negotiation, old clients and servers, missing profiles,
both workers, saved preferences, reset, recovery, and source-offset parity.
Do not send custom names through the current three-value punctuation enum.
