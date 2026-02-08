# Omnivox Development Roadmap

## Phase 1: Foundation (Week 1-2)

### Core Infrastructure
- [ ] Initialize Rust project with Cargo workspace
- [ ] Set up basic project structure
- [ ] Implement Emacspeak protocol parser
- [ ] Create command routing system
- [ ] Build state management (TtsState)
- [ ] Implement command queue system
- [ ] Add basic logging/debugging

### Testing Foundation
- [ ] Unit tests for command parser
- [ ] Unit tests for queue system
- [ ] Integration test framework

**Milestone:** Can parse all Emacspeak commands and queue them

## Phase 2: Single-Platform TTS (Week 3-4)

### Choose Starting Platform: Linux (easiest for development)
- [ ] Implement TTS trait
- [ ] Create Speech Dispatcher client
- [ ] Test basic text-to-speech
- [ ] Handle voice switching
- [ ] Implement rate/pitch/volume control

### Audio Pipeline Basics
- [ ] Set up cpal for audio output
- [ ] Create AudioBuffer type
- [ ] Implement PCM buffer handling
- [ ] Basic audio playback

**Milestone:** Can speak text on Linux with basic voice control

## Phase 3: Core Audio Effects (Week 5-6)

### Critical Effects
- [ ] Silence trimming (leading/trailing)
- [ ] Channel panning (left/right/both)
- [ ] Volume control (per audio type)
- [ ] Implement AudioEffect trait
- [ ] Build AudioPipeline processor

### Audio Quality
- [ ] Test silence detection thresholds
- [ ] Optimize buffer processing
- [ ] Measure and optimize latency

**Milestone:** Clean audio with trimming and panning

## Phase 4: Complete Emacspeak Protocol (Week 7-8)

### Remaining Commands
- [ ] Tone generation (`t` command)
- [ ] Audio icon playback (`a`, `p` commands)
- [ ] Letter speaking with pitch raise (`l` command)
- [ ] Silence insertion (`sh` command)
- [ ] All state management commands

### Voice Switching
- [ ] Inline code processing (`c` command)
- [ ] Voice profile switching
- [ ] Test rapid voice changes

**Milestone:** Full Emacspeak protocol compatibility on Linux

## Phase 5: macOS Support (Week 9-10)

### AVSpeechSynthesizer Integration
- [ ] Create Objective-C bindings (cacao or objc2)
- [ ] Implement MacOsTtsEngine
- [ ] Handle voice enumeration
- [ ] Test voice quality and selection
- [ ] Port SwiftMac voice configuration

### Platform-Specific Features
- [ ] Multi-device audio routing (macOS-specific)
- [ ] Runtime channel switching
- [ ] Audio device enumeration

**Milestone:** Feature parity with SwiftMac on macOS

## Phase 6: Windows Support (Week 11-12)

### SAPI Integration
- [ ] Research SAPI bindings (windows-rs)
- [ ] Implement WindowsTtsEngine
- [ ] Handle voice enumeration
- [ ] Test on Windows

**Milestone:** Working TTS on Windows

## Phase 7: Fallback Engine (Week 13)

### eSpeak-ng Integration
- [ ] Embed or link eSpeak-ng
- [ ] Implement EspeakEngine
- [ ] Fallback logic (try native, fall back to eSpeak)
- [ ] Test quality

**Milestone:** Universal fallback ensures it works everywhere

## Phase 8: Advanced Features (Week 14-15)

### Multi-Device Routing
- [ ] Device enumeration per platform
- [ ] Route speech/notifications/tones/sounds independently
- [ ] Environment variable configuration
- [ ] Runtime device switching

### Network Mode
- [ ] TCP listener (-p flag)
- [ ] Connection management
- [ ] Command buffering for network

**Milestone:** SwiftMac feature parity complete

## Phase 9: Sox-Style Effects (Week 16-17)

### Audio Effects
- [ ] Reverb effect
- [ ] Echo effect
- [ ] Chorus effect
- [ ] Tremolo effect
- [ ] Phaser effect
- [ ] Parse effect codes from commands

### Performance
- [ ] Optimize effect algorithms for speed
- [ ] Benchmark latency with effects
- [ ] Cache/reuse effect state

**Milestone:** Mac server feature parity

## Phase 10: Polish & Optimization (Week 18-19)

### Performance
- [ ] Profile hot paths
- [ ] Optimize buffer allocations
- [ ] Minimize copying
- [ ] Benchmark against SwiftMac
- [ ] Target < 100ms latency

### Error Handling
- [ ] Comprehensive error handling
- [ ] Graceful degradation
- [ ] Helpful error messages
- [ ] Recovery from TTS failures

### Documentation
- [ ] API documentation
- [ ] User guide
- [ ] Emacspeak integration guide
- [ ] Voice configuration examples

**Milestone:** Production-ready quality

## Phase 11: Testing & Validation (Week 20)

### Comprehensive Testing
- [ ] End-to-end Emacspeak integration tests
- [ ] Multi-platform testing
- [ ] Stress testing (rapid commands)
- [ ] Memory leak detection
- [ ] Long-running stability tests

### Real-World Usage
- [ ] Daily usage testing
- [ ] Bug fixes from real usage
- [ ] Performance tuning

**Milestone:** Stable, tested, ready for release

## Phase 12: Release (Week 21)

### Packaging
- [ ] Binary releases for macOS/Linux/Windows
- [ ] Installation scripts
- [ ] Homebrew formula (macOS)
- [ ] Integration with Emacspeak

### Documentation
- [ ] Release notes
- [ ] Migration guide from SwiftMac
- [ ] Known issues
- [ ] FAQ

**Milestone:** Version 1.0 release

## Future Enhancements (Post-1.0)

### Language Features
- [ ] Language switching tables
- [ ] Language aliases
- [ ] Per-language voice memory
- [ ] Multi-language document support

### Advanced Audio
- [ ] Audio streaming (reduce buffering)
- [ ] Real-time effect parameter adjustment
- [ ] Custom effect chains
- [ ] Audio format options (beyond PCM)

### Emacspeak Extensions
- [ ] New protocol commands for Rust-specific features
- [ ] Performance metrics reporting
- [ ] Voice profile management

## Success Metrics

### Performance Targets
- Latency: < 100ms command-to-audio
- Memory: < 50MB baseline
- CPU: < 5% idle, < 30% during speech

### Quality Targets
- No crashes (Rust safety)
- 100% Emacspeak protocol coverage
- Voice quality matches or exceeds SwiftMac
- Works on all three platforms

### Adoption Targets
- Replace SwiftMac in personal usage
- Community testing and feedback
- Integration into official Emacspeak distribution
