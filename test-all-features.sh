#!/bin/bash
# Comprehensive test of omnivox features

echo "=== Testing Omnivox Features ==="
echo ""

echo "Test 1: Version announcement"
echo "version" | ./target/release/omnivox 2>/dev/null &
PID=$!
sleep 2
kill $PID 2>/dev/null

echo ""
echo "Test 2: Basic speech"
echo "tts_say {Testing basic speech functionality}" | timeout 3 ./target/release/omnivox 2>/dev/null

echo ""
echo "Test 3: Queue and dispatch"
cat << 'EOF' | timeout 5 ./target/release/omnivox 2>/dev/null
q {First sentence.}
q {Second sentence.}
q {Third sentence.}
d
EOF

echo ""
echo "Test 4: Voice switching"
cat << 'EOF' | timeout 6 ./target/release/omnivox 2>/dev/null
c [{voice en-US:Samantha}]
q {Hello, I am Samantha.}
c [{voice en-US:Alex}]
q {And I am Alex.}
d
EOF

echo ""
echo "Test 5: Split caps"
cat << 'EOF' | timeout 4 ./target/release/omnivox 2>/dev/null
tts_split_caps 1
tts_say {CamelCaseText}
EOF

echo ""
echo "Test 6: Speech rate control"
cat << 'EOF' | timeout 6 ./target/release/omnivox 2>/dev/null
tts_set_speech_rate 0.3
tts_say {Speaking slowly.}
tts_set_speech_rate 0.7
tts_say {Speaking quickly.}
EOF

echo ""
echo "Test 7: Pitch control"
cat << 'EOF' | timeout 6 ./target/release/omnivox 2>/dev/null
tts_set_pitch_multiplier 0.8
tts_say {Low pitch voice.}
tts_set_pitch_multiplier 1.5
tts_say {High pitch voice.}
EOF

echo ""
echo "Test 8: Letter speaking"
cat << 'EOF' | timeout 5 ./target/release/omnivox 2>/dev/null
l A
l B
l C
EOF

echo ""
echo "=== All tests complete ==="
