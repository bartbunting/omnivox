# Piper companion third-party notices

The optional companion combines the Omnivox Piper helper with maintained
libpiper and its native runtime stack. These components retain their own
licences:

- Piper/libpiper v1.7.0 is GPL-3.0-or-later. Its licence and imported-source
  provenance are included as `Piper-GPL-3.0-or-later.txt` and
  `Piper-UPSTREAM.md`.
- eSpeak NG at the commit recorded in `SOURCE-PROVENANCE.json` is statically
  linked into libpiper. Its GPL text and the Apache, BSD, Unicode-data, UCD,
  Sonic, and NetBSD compatibility notices present in the build inputs are
  included in this directory.
- ONNX Runtime 1.22.0 is distributed with its upstream `LICENSE` and
  `ThirdPartyNotices.txt` files.

The Omnivox repository commit and `omnivox-Cargo.lock` recorded here identify
the Omnivox-authored helper and Rust dependency source. `LICENSING.md` in the
companion root provides the project-wide component map. This notice is an
engineering inventory, not legal advice.
