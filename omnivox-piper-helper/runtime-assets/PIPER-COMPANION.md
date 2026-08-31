# Omnivox Piper Companion

This directory is an optional native companion for a matching Omnivox build.
Keep the directory intact and install it as `piper/` beside the `omnivox`
executable. Omnivox discovers `piper/omnivox-piper-helper` automatically.
The platform archive extracts with this `piper/` directory already at its
root; extract it beside an existing generic Omnivox installation rather than
merging the directory contents into that installation.

The companion does not include a voice model. Supply a compatible `.onnx`
model and its adjacent `<model>.onnx.json` or `<model>.json` configuration with
`OMNIVOX_PIPER_MODEL` or `--piper-model`. Review the voice's `MODEL_CARD`
before use because model licences and restrictions vary.

`SOURCE-PROVENANCE.json` identifies the source and native inputs used for this
payload. `SHA256SUMS` covers every other regular file in this directory.
`LICENSE`, `LICENSING.md`, and `third-party-licenses/` describe the component
licensing boundaries; the root MIT licence applies only to Omnivox-authored
portions.

Preserve this complete directory when relocating, upgrading, or
redistributing the companion. Do not merge its `espeak-ng-data/` with the
different data directory shipped in the generic Omnivox archive.
