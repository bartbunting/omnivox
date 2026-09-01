# ADR 0003: Source-built RuTTS Companion

- Status: Accepted
- Date: 2026-09-01
- Supersedes: the provisional RuTTS runtime-supply decision in ADR 0001

## Context

ADR 0001 placed RuTTS behind an isolated helper but deferred its final
packaging policy until its source, licence, ABI, and portability had been
evaluated.

RuTTS v6.3.3 is a compact MIT-licensed C library. Its public API accepts
KOI8-R text, streams raw signed 8-bit mono PCM at 10 kHz through a callback,
and exposes rate, pitch, intonation, gap, and male/female voice controls. The
library has no required runtime data files and, when built without RuLex,
depends only on the C and math runtimes.

Upstream supplies source and Ubuntu packages rather than maintained binary
packages for every Omnivox target. Requiring a user-installed runtime would
therefore make the integration primarily Linux-specific despite the portable
core.

RuLex is a separate pronunciation dictionary, library, and database stack. It
improves lexical stress and exceptional pronunciations, but adds separate
licensing, provenance, locale, storage-backend, and platform concerns.

## Decision

### Publish RuTTS as a separate source-built companion

The RuTTS helper is built reproducibly from upstream v6.3.3 at commit
`2848d2892097320ed37fc963b439b15803f47f0c` and linked only into its own
process. Both upstream built-in voices are exposed as stable physical voices.

The companion remains separate from generic Omnivox archives. Its release
gate records the upstream archive and source-tree hashes, complete MIT licence,
corresponding source, locked Rust inputs, relocation checks, real synthesis,
cancellation, and clean shutdown.

The intended target matrix matches the portable Flite companion:

| Platform | Architecture |
|---|---|
| GNU/Linux | x86-64, ARM64 |
| macOS | Intel, Apple Silicon |
| Windows | x86-64, ARM64 |

A target is described as supported only after its native build and acceptance
checks pass. Compile-only evidence remains labelled as such.

### Keep the helper boundary

RuTTS remains a dedicated helper under ADR 0001. This contains native faults,
keeps its legacy 8-bit PCM and text encoding out of the main server, and lets
the helper host retire synthesis that does not stop within the cancellation
grace period.

The adapter converts Unicode input to KOI8-R, rejects embedded nulls, bounds
native output, discards PCM delivered after cancellation, converts samples to
Omnivox's canonical PCM format, and serializes access even though initial
concurrency probes found no shared mutable synthesis state.

### Defer RuLex

The v1.6 RuTTS companion is built without RuLex. It does not contain or load a
RuLex library or database. Pronunciation-dictionary support requires a later
licensing, provenance, API, and cross-platform decision; it must not be added
implicitly through host discovery.

## Consequences

- Users get the same two self-contained Russian voices on every accepted
  companion target without installing a native RuTTS package.
- Omnivox must maintain six companion builds, a corresponding-source artifact,
  and release verification alongside the existing Flite artifacts.
- The helper performs a deliberate Unicode-to-KOI8-R conversion because the
  upstream API does not accept UTF-8.
- Russian words with ambiguous or exceptional stress may be pronounced less
  accurately until RuLex support is separately designed.
- The upstream callback may deliver a final partial buffer after cancellation;
  the adapter must discard it and the helper process remains the hard stop.

## Alternatives considered

### Load a user-installed RuTTS library

Rejected as the default. It would reduce companion packaging but official
install guidance and packages are Linux-oriented, the API has no runtime
version query, and users on macOS and Windows would need unsupported manual
builds.

### Bundle RuLex with the first companion

Deferred. RuLex adds an LGPL library, LMDB or Berkeley DB, a separately built
dictionary, KOI8-R locale behavior, and additional provenance work that is not
required for functional RuTTS speech.

### Build RuTTS into the main server

Rejected for the initial integration. It would conflict with ADR 0001's rule
that new native and not-yet-established engines begin in isolated helpers, and
offers little benefit for this small callback-based engine.
