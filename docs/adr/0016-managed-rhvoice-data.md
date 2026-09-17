# ADR 0016: Managed RHVoice voice and language data

Status: Accepted direction under the maintainer's request for RHVoice downloads
and uninstallation on 2026-09-17.

Extend ADR 0012's existing local acquisition, validation, activation and removal
services to RHVoice. The first catalogue contains Alan, Bdl, Clb and Ksp English
voices. Emacsvox owns the reviewed catalogue; Omnivox owns native operations.
The RHVoice runtime remains separately installed under ADR 0002. Neither project
redistributes that runtime or voice assets in executable releases.

Each package owns a checksum-pinned copy of its voice and English language files,
plus upstream notices. Use pinned HTTPS file URLs and a bounded allowlist of
resource paths, without an archive extractor or new dependency. Language data
is intentionally duplicated between packages: removing one package cannot remove
another's prerequisite. Enabled packages must agree on their English data.

Catalogue, installed-index and runtime schema 3 add RHVoice. Earlier schemas
remain valid and retain their serialization. Downloads start disabled. Managed
resources reach the isolated RHVoice helper only after explicit Apply; eligibility
and both-lane acknowledgement retain their existing rules. Ordinary RHVoice
startup preserves external data discovery and rejects a managed voice colliding
with an externally installed physical ID. External voice exclusions still apply.

Disposable native validation uses only the selected package, suppressing external
data and configuration. It must discover exactly the requested voice and produce
PCM before installation. Managed acquisition requires an explicit compatible
`OMNIVOX_RHVOICE_LIBRARY`. Evidence schema 2 observes the selected helper and
external runtime library before and after validation, alongside verified data.
This is observation, not attestation of system DLLs or permission to reuse old
validation. The existing supervisor owns timeouts, cancellation and cleanup.

Uninstallation checks exact ownership of nested resources, rejects unlisted files
and links, and retains active, rollback and unretired-session references. It never
deletes external data, the RHVoice library or saved palette choices. Old runtimes
do not advertise the provider and reject schema 3 before activation. Native
Windows and Linux acceptance remain separate; this adds no macOS support claim.
