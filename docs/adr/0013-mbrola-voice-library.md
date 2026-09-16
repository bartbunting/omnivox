# ADR 0013: MBROLA English voice library

- Status: Accepted for the explicitly configured development companion
- Date: 2026-09-17

## Decision

Extend the existing separate MBROLA helper with the voice-library lifecycle in
ADR 0012. The core never links MBROLA or the GPL frontend. Keep the explicit
absolute OMNIVOX_MBROLA_HELPER opt-in and private builder; this does not add a
generic release artifact, dependency, automatic runtime installer, or macOS
support claim. Runtime publication remains separate work.

The initial set is the existing en1 and optional US1, US2, US3 databases from
MBROLA-voices revision fe05a0ccef6a941207fd6aaad0b31294a1f93a51. Exact physical
IDs bind a reviewed database hash, sample rate and pinned frontend profile.
No arbitrary aliases, command arguments, or executable downloads cross the
catalogue boundary. Each download retains its individual licence and README.
Database terms remain distinct from the AGPL runtime and GPL frontend; these
assets are not relicensed or included in the generic core release.

Omnivox owns acquisition, native validation, immutable packages, enablement and
activation. Emacsvox supplies reviewed catalogue metadata and the existing
Download, Enable, Apply interface. Downloads start disabled. On first MBROLA
installation, retain en1 as an enabled built-in index row unless already
explicitly disabled. Preserve its existing physical ID. No speech restart or
palette edit occurs during installation.

Schema 2 of the catalogue, management index and runtime generation adds a
bounded MBROLA load set, including built-in en1. Version 1 remains valid for
existing providers. MBROLA-bearing inputs require version 2, as required by
ADR 0012’s extension rule.
Older generation bytes remain valid and serialize unchanged when no MBROLA
section is present. Old binaries reject new MBROLA inputs before activation.
Validation uses the exact staged companion and selected database, including a
real synthesis without playback. Apply retains the existing two-lane preflight,
acknowledgement, commit and rollback rules. Disabled voices cannot reappear via
aliases, fallback, preview or restart.

The helper retains metadata only. Each serialized synthesis verifies the small
shared English frontend bundle and selected database, opens that database in a
short-lived native child, and retires the child after bounded buffered PCM.
Other downloaded databases are neither scanned nor loaded per utterance. Keep
cancellation ownership, final text-byte preservation, and truthful no-marker /
no-streaming capabilities. Adding voices must be checked against en1 latency.

## Consequences

Users can manage MBROLA voices through the same library as Piper and Flite,
without an additional downloader or resident database cache. The prototype
runtime is still installed separately. Promotion to ordinary distribution,
additional languages, streaming and markers require their own acceptance.
