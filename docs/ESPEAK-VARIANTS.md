# eSpeak NG voice variants

Bundled variants are available on demand for exact preview and ordinary palette
speech. Choose a base voice and variant; no enabling or restart is required.
Existing physical IDs remain unchanged, for example `espeak:gmw/en-US+m1`.
Use the actual speech host's IDs; Windows can report different path separators.

A running worker advertises `espeak_variants_v1` and includes the bounded
`espeak_variants` catalogue in its eSpeak descriptor. Base voices stay in the
ordinary inventory. Exact selection validates and derives only the requested
combination, so discovery never expands the complete Cartesian product.
Automatic and property selectors continue using the ordinary base inventory.

The diagnostic `omnivox --list-espeak-variants` remains available, returning
schema-1 JSON with bases, variants and native limits. No files are downloaded.
The old `OMNIVOX_ESPEAK_VARIANTS` startup setting remains readable: enabled
entries can add explicit inventory rows, including unavailable missing choices.
Its 64-entry/16-KiB limits apply only to that legacy setting. An omitted or
previously disabled combination is now usable when its base and variant exist.
Saved palette references need no conversion.

Engine disablement and explicit voice-library physical exclusions still apply.
Variant selection and synthesis use the existing native lock. Each subsequent
request selects its own voice, so auditioning does not change ordinary routing.
Native identity is checked before accepting audio: a missing variant must not
silently fall back to its base while claiming a successful exact preview.
Buffered and progressive synthesis retain rate, pitch, markers and cancellation.

The pinned native library has a 40-byte combined identifier. Omnivox rejects
combinations longer than 39 native bytes, numeric aliases, traversal and nested
variants. The new descriptor field defaults to absent for old descriptors and
other engines. Old servers lack the negotiated capability and cannot promise
on-demand combinations. Updating the software requires new workers once.

`python3 tools/verify_espeak_variants.py PATH_TO_OMNIVOX` exercises two owned
null-output workers with no configured combinations. It checks compact live
inventory, both exact previews, base switching, palette registration and complete
previews, ordinary speech with exact playback markers, missing-voice fallback,
engine disablement and unchanged worker PIDs.
For a separate data tree, pass `--espeak-data NATIVE_PARENT`. These checks use
null output; they do not establish audible acceptance or support on untested
platforms. See [ADR 0014](adr/0014-on-demand-espeak-variants.md).
