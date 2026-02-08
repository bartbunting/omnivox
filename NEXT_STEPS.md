# Next Steps for Omnivox

## What We've Accomplished

### Planning & Documentation ✓
- **GOALS.md** - Complete project vision and requirements
  - Cross-platform TTS with native OS libraries + eSpeak-ng fallback
  - Mandatory audio processing pipeline (trim, pan, volume, effects)
  - Performance-first design (latency < 100ms)
  - Intra-sentence voice switching
  - Full Emacspeak protocol support
  - Implementation language: Rust

- **ARCHITECTURE.md** - Detailed technical design
  - Component architecture diagram
  - Core types and traits
  - Concurrency model (Tokio)
  - Platform-specific TTS abstractions
  - Audio pipeline design
  - Performance optimization strategies

- **ROADMAP.md** - 21-week development plan
  - Phase 1-4: Foundation + Linux TTS + Core effects + Full protocol
  - Phase 5-7: macOS, Windows, eSpeak-ng support
  - Phase 8-9: Multi-device routing + Sox effects
  - Phase 10-12: Polish, testing, release

### Clean Slate ✓
- Removed experimental C code (omnivox.c)
- Cleaned up legacy build artifacts (CMake, dictionaries, .wav files)
- Updated .gitignore for Rust project
- Created Rust-focused Makefile

## Immediate Next Steps

### 1. Initialize Rust Project

```bash
cd /Users/rmelton/projects/robertmeta/omnivox

# Create Cargo workspace
cargo new --lib omnivox-core
cargo new --lib omnivox-tts
cargo new --bin omnivox-cli

# Create workspace Cargo.toml
cat > Cargo.toml << 'EOF'
[workspace]
members = [
    "omnivox-core",
    "omnivox-tts",
    "omnivox-cli",
]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
cpal = "0.15"
thiserror = "1"
EOF
```

### 2. Implement Command Parser (Phase 1)

Start in `omnivox-core/src/lib.rs`:

```rust
// Command parsing
pub mod command;
pub mod queue;
pub mod state;

pub use command::{Command, CommandId, ParseError};
pub use queue::{CommandQueue, QueueItem};
pub use state::{TtsState, AudioRouting, ChannelMode};
```

Create `omnivox-core/src/command.rs` with:
- `parse_command()` function
- Regex pattern for Emacspeak protocol
- Unit tests

### 3. Write First Tests

Create test cases for command parsing:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_queue_command() {
        let cmd = parse_command("q {Hello World}").unwrap();
        assert_eq!(cmd.id, CommandId::Queue);
        assert_eq!(cmd.args, Some("Hello World".to_string()));
    }

    #[test]
    fn parse_dispatch_command() {
        let cmd = parse_command("d").unwrap();
        assert_eq!(cmd.id, CommandId::Dispatch);
        assert!(cmd.args.is_none());
    }
}
```

### 4. Set Up Development Environment

Install Rust tools:
```bash
# Clippy for linting
rustup component add clippy

# Rustfmt for formatting
rustup component add rustfmt

# Cargo-watch for auto-rebuild
cargo install cargo-watch
```

### 5. Create CLAUDE.md for Omnivox

Document the Rust project structure and development workflow for future Claude Code sessions.

## Questions to Resolve

Before starting implementation:

1. **Audio Library Choice:**
   - Start with `cpal` for cross-platform output?
   - Or investigate platform-specific APIs first?

2. **TTS Bindings Approach:**
   - For macOS: Use `objc2` or `cacao` for AVSpeechSynthesizer?
   - For Linux: Use `libspeechd` or direct D-Bus?
   - For Windows: Use `windows-rs` for SAPI?

3. **Development Priority:**
   - Start with Linux (easiest) or macOS (existing SwiftMac as reference)?

4. **Testing Strategy:**
   - Run tests against actual Emacspeak?
   - Or create mock Emacspeak command sequences?

## Recommended First Session Plan

1. Initialize Cargo workspace (30 min)
2. Implement command parser (2 hours)
3. Write comprehensive parser tests (1 hour)
4. Implement basic queue system (1 hour)
5. Set up CI/CD skeleton (30 min)

**Goal:** Have a working command parser that can recognize all Emacspeak commands.

## Resources

### Rust Crates to Investigate
- **Audio:** `cpal`, `rodio`, `dasp`
- **Async:** `tokio`, `async-std`
- **Platform bindings:** `objc2`, `windows-rs`, `nix`
- **TTS:** Research existing crates or FFI approach
- **DSP:** `rubato`, `biquad`, custom implementations

### SwiftMac Reference Points
- `main.swift`: Command processing loop
- `statestore.swift`: State management pattern
- Protocol command mapping around line 522-553

### Emacspeak Protocol Reference
- `emacspeak/servers/tts-lib.tcl`: Common protocol functions
- `emacspeak/servers/espeak`: Full implementation example
- `emacspeak/servers/mac`: Python implementation with sox effects

## Success Criteria for Phase 1

By end of Week 2, we should have:
- ✓ Cargo workspace set up
- ✓ Command parser with 100% test coverage
- ✓ Queue system implemented
- ✓ State management (TtsState struct)
- ✓ Basic stdin command loop
- ✓ All commands recognized (even if not implemented)

This creates the foundation for Phase 2 (TTS integration).
