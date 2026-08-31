# Vendored Piper libpiper Source

This directory contains the native `libpiper` source selected for Omnivox's
optional Piper helper migration. It is third-party source, not MIT-licensed
Omnivox-authored code.

## Provenance

- Upstream: <https://github.com/OHF-Voice/piper1-gpl>
- Upstream tag: `v1.7.0`
- Upstream commit: `7b8e8f7197a480047677715f00d3d78903b55a2a`
- Imported paths: `COPYING`, `setup.py`, and the complete `libpiper/` subtree
- Imported by Omnivox: 2026-08-31

The imported files are byte-for-byte copies from that commit. This
`UPSTREAM.md` file is the only Omnivox-authored file in this directory.
`setup.py` is retained because the upstream `libpiper/CMakeLists.txt` reads it
to obtain the Piper version; Omnivox does not use the Python package as its
runtime integration.

## Licence

Upstream declares this source under GPL-3.0-or-later. The complete upstream
GPL text is preserved in [COPYING](COPYING). Vendoring this component does not
relicense it under Omnivox's root MIT licence.

## Scope of this import

This import pins libpiper itself. Its upstream build also identifies a pinned
eSpeak NG commit and ONNX Runtime version, but the pristine CMake file can
still fetch those inputs without verified digests. Omnivox release builds must
provide separately pinned, checksum-verified inputs and must not rely on that
implicit network behavior. The Linux x64 build now overlays only its generated
source copy, supplying the eSpeak NG, Sonic, and ONNX Runtime archives locked
in `omnivox-piper-sys/native-inputs.json`; the vendored files in this directory
remain unchanged. Windows x64 and macOS ARM64/x64 now have their own verified
input locks as well, but each platform still requires a successful native
build and runtime acceptance before release support is claimed.

The old `omnivox-piper-sys/piper/` path remains ignored so an existing legacy
developer checkout does not become an accidental Git addition. It is no longer
read by the active build.
