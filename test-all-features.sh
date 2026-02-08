#!/bin/bash
# Comprehensive test of omnivox features
# Requires: built binary at ./target/release/omnivox

set -e

OMNIVOX="./target/release/omnivox"

if [ ! -f "$OMNIVOX" ]; then
    echo "Binary not found. Run 'make build' first."
    exit 1
fi

echo "=== Testing Omnivox Features ==="
echo ""

echo "Test 1: Version announcement"
echo "version" | timeout 3 "$OMNIVOX" 2>/dev/null || true
sleep 1

echo ""
echo "Test 2: Basic speech"
echo "tts_say {Testing basic speech functionality}" | timeout 5 "$OMNIVOX" 2>/dev/null || true
sleep 1

echo ""
echo "Test 3: Queue and dispatch"
cat << 'EOF' | timeout 8 "$OMNIVOX" 2>/dev/null || true
q {First sentence.}
q {Second sentence.}
q {Third sentence.}
d
EOF
sleep 1

echo ""
echo "Test 4: Voice switching"
cat << 'EOF' | timeout 8 "$OMNIVOX" 2>/dev/null || true
c [{voice en-US:Samantha}]
q {Hello, I am Samantha.}
c [{voice en-US:Alex}]
q {And I am Alex.}
d
EOF
sleep 1

echo ""
echo "Test 5: Split caps"
cat << 'EOF' | timeout 4 "$OMNIVOX" 2>/dev/null || true
tts_split_caps 1
tts_say {CamelCaseText}
EOF
sleep 1

echo ""
echo "Test 6: Speech rate control"
cat << 'EOF' | timeout 8 "$OMNIVOX" 2>/dev/null || true
tts_set_speech_rate 0.3
tts_say {Speaking slowly.}
tts_set_speech_rate 0.7
tts_say {Speaking quickly.}
EOF
sleep 1

echo ""
echo "Test 7: Pitch control"
cat << 'EOF' | timeout 8 "$OMNIVOX" 2>/dev/null || true
tts_set_pitch_multiplier 0.8
tts_say {Low pitch voice.}
tts_set_pitch_multiplier 1.5
tts_say {High pitch voice.}
EOF
sleep 1

echo ""
echo "Test 8: Letter speaking"
cat << 'EOF' | timeout 5 "$OMNIVOX" 2>/dev/null || true
l A
l b
l C
EOF
sleep 1

echo ""
echo "Test 9: Tone generation"
cat << 'EOF' | timeout 5 "$OMNIVOX" 2>/dev/null || true
t 440 100
d
t 880 100
d
EOF
sleep 1

echo ""
echo "Test 10: Punctuation levels"
cat << 'EOF' | timeout 8 "$OMNIVOX" 2>/dev/null || true
tts_set_punctuations all
tts_say {Hello, world! How are you?}
tts_set_punctuations none
tts_say {Hello, world! How are you?}
EOF
sleep 1

echo ""
echo "Test 11: espeak-ng engine"
cat << 'EOF' | OMNIVOX_ENGINE=espeak timeout 5 "$OMNIVOX" 2>/dev/null || true
tts_say {This is the espeak engine speaking.}
EOF
sleep 1

echo ""
echo "=== All tests complete ==="
