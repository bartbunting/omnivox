# eSpeak NG voice variants

The development adapter can expose selected bundled variants as physical voices.
Existing base voice IDs and the default voice are unchanged. A combination uses
the exact native base ID followed by `+` and the variant file identifier, such as
`espeak:gmw/en-US+m1`. Copy IDs from the actual speech host: Windows can report
different path separators.

`omnivox --list-espeak-variants` silently returns schema-1 JSON with `bases`,
`variants`, `max_choices` and `max_native_identifier_bytes`. Discovery queries
the currently selected eSpeak data, without using the base-voice cache for
variants. It never enumerates every possible combination or downloads files.

Set `OMNIVOX_ESPEAK_VARIANTS` to a JSON array before starting speech:

```json
[{"base_voice_id":"espeak:gmw/en-US","variant_id":"m1","enabled":true}]
```

The array is limited to 64 distinct combinations and 16 KiB. Disabled entries
may be retained with `enabled:false`; they are omitted from runtime inventory
and cannot be selected by ordinary speech, fallback or exact preview. Saved
palette references remain unchanged. Enabled choices with a missing base or
variant are reported unavailable, while existing base voices remain usable.
The ordinary voice-library physical exclusions and engine disablement also apply.

Settings affect new workers only. Restart both speech lanes explicitly to apply
the selection, then refresh inventory and use the existing exact preview and
palette editor. A running worker keeps its startup selection. Native identity
is checked before accepting audio, so a disappeared variant cannot silently
speak with the base voice while reporting success. Both buffered and progressive
synthesis preserve the variant identity and existing rate, pitch and marker paths.

The pinned native library uses a 40-byte combined identifier. Omnivox rejects
combinations longer than 39 native bytes, numeric aliases and path traversal
before native selection. This first implementation manages bundled availability
through startup settings; it does not install external variant files or add
variant packages to the Piper/Flite installed-library schema.

Native unit checks cover discovery, exact PCM identity, switching back to the
base, rate/pitch changes, buffered/progressive rejection and native fallback on
a missing file. `python3 tools/verify_espeak_variants.py PATH_TO_OMNIVOX` checks
two owned null-output workers, ordinary inventory/eligibility, exact previews,
base switching, disabled rejection and full-chain fallback. Linux passed with
141 base voices and 103 variants, alongside the locked workspace tests, workspace
Clippy, formatting, documentation links and supported `make dev` staging.
Native Windows and audible acceptance remain separate checks.
