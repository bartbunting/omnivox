# ADR 0012: TGSpeechBox default distribution

- Status: Accepted
- Accepted: 2026-10-09
- Partially supersedes: The TGSpeechBox separate experimental, Windows-only
  distribution restriction in [ADR 0001](0001-engine-isolation-and-distribution.md).
  Its process isolation and licensing requirements remain in force.

## Context

TGSpeechBox provides compact formant speech and native voice controls. Requiring
a separate installation adds setup work despite its modest payload size. The
distribution policy can include this engine while preserving its isolated native
runtime and the existing choice of preferred speech engine.

## Decision

Include TGSpeechBox by default in normal local builds, every generic Omnivox
archive, Debian packages and the Emacsvox Windows runtime. Include the complete
helper, profiles, language packs, phonemizer data, inventories, notices and
provenance. Remove its experimental designation. Bundling makes the engine
available without changing the user's preferred engine or voice.

Target Linux, macOS and Windows on x64 and ARM64 for the helper. This does not
add new core-server package targets: each existing generic package includes its
matching helper. Retain standalone companion archives for compatibility.

Keep TGSpeechBox's eSpeak frontend and native DSP in their dedicated helper
process. Moving them into the server requires a separate decision. Preserve
pinned source inputs, reproducible toolchains, complete notices and exact
corresponding source. The combined helper remains GPL-3.0-or-later; the narrow
Omnivox boundary and upstream TGSpeechBox source retain their own notices.

The clean Emacsvox Windows build compiles the helper in its pinned release
container. Host staging verifies those outputs and generates inventories using
that exact Windows helper. Development builds may explicitly omit TGSpeechBox;
clean release bundles require it.

Qualify each target with native checks before publication. Verify architecture,
complete payloads, checksums, inventories, notices and corresponding source.
Relocate the package and test discovery and non-silent synthesis through its
actual bundled helper and server, including streaming, common and native voice
controls, reset, cancellation and shutdown. Do not replace the bundled helper
with a separate companion before testing it. Matrix entries and cross-compilation
alone do not establish runtime support. Failed or missing native acceptance
blocks publication for that target.

Other engines retain their existing distribution policies. Managed acquisition
continues to follow [ADR 0007](0007-managed-voice-lifecycle.md).

## Consequences

Packages grow and each release gains native verification work. In return,
TGSpeechBox is available immediately after installing Omnivox. Helper isolation
continues to contain native failures and conflicting dependencies.

Exact contents and installation instructions belong to the
[engine guide](../engines/tgspeechbox.md) and
[licensing map](../LICENSING.md). Current qualification belongs to
[project status](../STATUS.md); published measurements belong in the
[evidence archive](../benchmarks/README.md).
Acceptance of this decision does not establish platform qualification or release
publication.
