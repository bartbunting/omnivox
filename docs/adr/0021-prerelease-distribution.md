# ADR 0021: Prerelease distribution

- Status: Accepted for implementation
- Accepted: 2026-11-14; explicit approval of the prerelease policy and its record.
- Related: [Engine distribution](0001-engine-isolation-and-distribution.md),
  [bundled TGSpeechBox](0012-tgspeechbox-default-distribution.md).

## Context

Native changes need an installable testing release before stable acceptance.
Stable-only tags and the ordinary Homebrew formula would make that candidate
the default upgrade for existing users. The existing component distribution
decisions govern payloads and verification, but do not define a testing channel.

## Decision

Use `vMAJOR.MINOR.PATCH-pre.N` tags, starting at positive integer `N = 1`,
with the corresponding SemVer in the workspace, binaries and archive names.
Each published tag and its assets are immutable; corrections use a new number.
Published changelog sections remain frozen. Debian substitutes `~pre.N` for
`-pre.N` in its package version and filename, so the eventual stable version
sorts after every prerelease.

Retain the complete stable release asset set, checksums, corresponding source,
licensing and native verification gates. A prerelease is first uploaded as a
draft; only successful required gates permit publication. Resume verification
against the same tag and assets with the same prerelease classification.
Publish as a GitHub prerelease explicitly excluded from Latest.

Offer the verified Mac archives through a separate `omnivox-pre` formula in
the project's Homebrew tap. Make it keg-only so it coexists with stable
`omnivox` without changing the ordinary command. Testers select its explicit
opt-prefix executable. Keep the stable updater restricted to published stable
releases; the prerelease updater must require a published numbered prerelease,
verify both Mac archives, and reject downgrades or changed same-version assets.
Require native Apple Silicon and Intel formula checks before merging, then
verify the merged formula against the exact published version.

Release notes identify the candidate client commit when a feature needs paired
Emacsvox changes. This does not constitute an Emacsvox release. Native synthesis
checks do not establish audible or physical-device acceptance: record those
remaining tester checks explicitly.

## Consequences and alternatives

Testers can install and remove the candidate alongside stable, at the cost of
an additional formula and full release verification. Updating the stable formula
would expose ordinary users to an unaccepted candidate. An unversioned head
build would lose immutable provenance and exact archive verification. Skipping
native gates for a prerelease would weaken existing distribution constraints.

The [deployment guide](../../.github/DEPLOYMENT.md#prereleases) owns the commands;
release evidence records actual publication and platform acceptance.
