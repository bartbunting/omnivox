# Next Steps for Omnivox

## Immediate: Homebrew Formula

Create a Homebrew tap for easy installation:

```bash
brew install robertmeta/omnivox/omnivox
```

The formula should:

- Build the Rust binary from source (or fetch a prebuilt release)
- Install `omnivox-voices.el` to a predictable location
- Print post-install instructions for Emacs setup

## Short Term: Linux Speech Dispatcher Backend

Create `omnivox-tts/src/linux.rs` implementing `TtsEngine`:

- Use `speech-dispatcher` or `ssip-client` crate for Speech Dispatcher
- Or use libspeechd C bindings via FFI
- espeak-ng already works as fallback on Linux
- Key requirement: must synthesize to memory buffer, not direct audio

## Additional Features (Lower Priority)

- Network mode (-p flag for TCP listener)
- Multi-device audio routing
- Sox-style effects (reverb, echo, chorus)
- Language switching tables
- Smart text chunking on sentence/clause boundaries
- Unify the two AudioBuffer types (omnivox-tts vs omnivox-audio)

## Reference

See CLAUDE.md for full architecture details and implementation patterns.
