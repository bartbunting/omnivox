#!/bin/bash
# Comprehensive audio test of omnivox: protocol commands, all engines,
# speech rates, and channel routing.
#
# Usage:
#   ./test-all-features.sh              # use installed omnivox binary
#   OMNIVOX_BIN=./target/release/omnivox ./test-all-features.sh
#   PIPER_MODEL=/path/to/model.onnx ./test-all-features.sh

OMNIVOX="${OMNIVOX_BIN:-omnivox}"
PIPER_MODEL="${PIPER_MODEL:-$HOME/piper-models/en_US-lessac-medium.onnx}"
PIPER_HELPER="${OMNIVOX_PIPER_HELPER:-}"

# Resolve to absolute path if given as relative
if [[ "$OMNIVOX" != omnivox && ! -f "$OMNIVOX" ]]; then
    echo "Binary not found: $OMNIVOX"
    echo "Run 'make build' or 'make build-piper', or set OMNIVOX_BIN."
    exit 1
fi

# Detect whether the model and adjacent/explicit helper are available.
if [ -z "$PIPER_HELPER" ]; then
    omnivox_path="$(command -v "$OMNIVOX" 2>/dev/null || true)"
    if [ -z "$omnivox_path" ] && [[ "$OMNIVOX" == */* ]]; then
        omnivox_path="$OMNIVOX"
    fi
    if [ -n "$omnivox_path" ]; then
        PIPER_HELPER="$(dirname "$omnivox_path")/omnivox-piper-helper"
        if [ ! -f "$PIPER_HELPER" ] && [ -f "$PIPER_HELPER.exe" ]; then
            PIPER_HELPER="$PIPER_HELPER.exe"
        fi
    fi
fi
HAS_PIPER=false
PIPER_SKIP_REASON="no model at $PIPER_MODEL"
if [ -f "$PIPER_MODEL" ] && [ -f "$PIPER_HELPER" ]; then
    HAS_PIPER=true
elif [ -f "$PIPER_MODEL" ]; then
    PIPER_SKIP_REASON="no helper at ${PIPER_HELPER:-<unresolved>}"
fi

pass=0
fail=0

# ---------------------------------------------------------------------------
# Helper: run a protocol session, print result.
#
#   run_test LABEL WAIT_SECS [ENV=VAL ...] <<'EOF'
#   protocol commands...
#   EOF
#
# WAIT_SECS: how long to sleep inside the pipe to keep stdin open while
#            audio plays.  Must be long enough for synthesis + playback.
# ---------------------------------------------------------------------------
run_test() {
    local label="$1"
    local wait="$2"
    shift 2
    local env_args=("$@")

    printf "  %-55s" "$label"

    local input
    input="$(cat)"

    if (printf "%s\n" "$input"; sleep "$wait") | \
        env "${env_args[@]}" timeout $((wait + 5)) "$OMNIVOX" 2>/dev/null
    then
        echo " OK"
        ((pass++))
    else
        echo " FAIL (exit $?)"
        ((fail++))
    fi
}

# ---------------------------------------------------------------------------
# Section 1: Protocol command coverage (native engine)
# ---------------------------------------------------------------------------
echo "=== Section 1: Protocol Commands (native engine) ==="
echo ""

run_test "1.1  tts_say basic" 4 <<'EOF'
tts_say {Testing basic speech functionality.}
EOF

run_test "1.2  Queue and dispatch" 6 <<'EOF'
q {First sentence.}
q {Second sentence.}
q {Third sentence.}
d
EOF

run_test "1.3  Pitch control via queue" 8 <<'EOF'
c [[pitch 1.5]]
q {High pitch.}
d
c [[pitch 0.7]]
q {Low pitch.}
d
EOF

run_test "1.4  Letter speaking" 4 <<'EOF'
l A
l B
l C
EOF

run_test "1.5  Tone generation" 4 <<'EOF'
t 440 200
d
t 880 200
d
t 1320 200
d
EOF

run_test "1.6  Punctuation: all" 4 <<'EOF'
tts_set_punctuations all
tts_say {Hello, world! Cost: $5.00. Ready?}
EOF

run_test "1.7  Punctuation: none" 4 <<'EOF'
tts_set_punctuations none
tts_say {Hello, world! Cost: $5.00. Ready?}
EOF

run_test "1.8  Split caps" 4 <<'EOF'
tts_split_caps 1
tts_say {CamelCaseIdentifier}
EOF

run_test "1.9  Stop command" 3 <<'EOF'
tts_say {This is a long sentence that should be interrupted before it finishes.}
s
tts_say {Stop worked.}
EOF

# ---------------------------------------------------------------------------
# Section 2: Engine comparison — same text on all three engines
# ---------------------------------------------------------------------------
echo ""
echo "=== Section 2: Engine Comparison ==="
echo ""

run_test "2.1  Native engine" 5 <<'EOF'
tts_say {This is the native platform voice speaking at normal speed.}
EOF

run_test "2.2  Espeak-ng engine" 5 \
    OMNIVOX_ENGINE=espeak <<'EOF'
tts_say {This is the espeak engine speaking at normal speed.}
EOF

if $HAS_PIPER; then
    run_test "2.3  Piper neural engine" 8 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_say {This is the piper neural voice speaking at normal speed.}
EOF
else
    echo "  2.3  Piper neural engine                                SKIPPED ($PIPER_SKIP_REASON)"
fi

# ---------------------------------------------------------------------------
# Section 3: Speech rate tests — slow / normal / fast per engine
#
# Rate scale: 0-100 integer, where 50 = normal, lower = faster, higher = slower.
# ---------------------------------------------------------------------------
echo ""
echo "=== Section 3: Speech Rate Tests ==="
echo ""

echo "  --- Native ---"
run_test "3.1  Native slow   (rate 25)" 6 <<'EOF'
tts_set_speech_rate 25
tts_say {This is native speech at a slow rate. Notice the pace.}
EOF

run_test "3.2  Native normal (rate 50)" 5 <<'EOF'
tts_set_speech_rate 50
tts_say {This is native speech at normal speed. Notice the pace.}
EOF

run_test "3.3  Native fast   (rate 75)" 4 <<'EOF'
tts_set_speech_rate 75
tts_say {This is native speech at a fast rate. Notice the pace.}
EOF

echo ""
echo "  --- Espeak ---"
run_test "3.4  Espeak slow   (rate 25)" 6 \
    OMNIVOX_ENGINE=espeak <<'EOF'
tts_set_speech_rate 25
tts_say {This is espeak at a slow rate. Notice the pace.}
EOF

run_test "3.5  Espeak normal (rate 50)" 5 \
    OMNIVOX_ENGINE=espeak <<'EOF'
tts_set_speech_rate 50
tts_say {This is espeak at normal speed. Notice the pace.}
EOF

run_test "3.6  Espeak fast   (rate 75)" 4 \
    OMNIVOX_ENGINE=espeak <<'EOF'
tts_set_speech_rate 75
tts_say {This is espeak at a fast rate. Notice the pace.}
EOF

if $HAS_PIPER; then
    echo ""
    echo "  --- Piper ---"
    run_test "3.7  Piper slow      (rate 25)" 10 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_set_speech_rate 25
tts_say {This is piper at a slow rate. Notice the pace.}
EOF

    run_test "3.8  Piper normal    (rate 50)" 9 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_set_speech_rate 50
tts_say {This is piper at normal speed. Notice the pace.}
EOF

    run_test "3.9  Piper fast      (rate 75)" 8 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_set_speech_rate 75
tts_say {This is piper at a fast rate. Notice the pace.}
EOF

    run_test "3.10 Piper very fast (rate 120)" 7 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_set_speech_rate 120
tts_say {Piper above one hundred. Notice the pace.}
EOF

    run_test "3.11 Piper max fast  (rate 150)" 6 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" <<'EOF'
tts_set_speech_rate 150
tts_say {Piper at one fifty. Very fast.}
EOF
else
    echo "  3.7-3.11 Piper rate tests                              SKIPPED"
fi

# ---------------------------------------------------------------------------
# Section 4: Channel routing — left / right / both
# ---------------------------------------------------------------------------
echo ""
echo "=== Section 4: Channel Routing ==="
echo ""

echo "  --- Native ---"
run_test "4.1  Native left channel" 4 \
    OMNIVOX_AUDIO_TARGET=left <<'EOF'
tts_say {Left ear only. Left ear only.}
EOF

run_test "4.2  Native right channel" 4 \
    OMNIVOX_AUDIO_TARGET=right <<'EOF'
tts_say {Right ear only. Right ear only.}
EOF

run_test "4.3  Native both channels" 4 \
    OMNIVOX_AUDIO_TARGET=both <<'EOF'
tts_say {Both ears. Both ears.}
EOF

echo ""
echo "  --- Espeak ---"
run_test "4.4  Espeak left channel" 4 \
    OMNIVOX_ENGINE=espeak OMNIVOX_AUDIO_TARGET=left <<'EOF'
tts_say {Left ear only. Left ear only.}
EOF

run_test "4.5  Espeak right channel" 4 \
    OMNIVOX_ENGINE=espeak OMNIVOX_AUDIO_TARGET=right <<'EOF'
tts_say {Right ear only. Right ear only.}
EOF

run_test "4.6  Espeak both channels" 4 \
    OMNIVOX_ENGINE=espeak OMNIVOX_AUDIO_TARGET=both <<'EOF'
tts_say {Both ears. Both ears.}
EOF

if $HAS_PIPER; then
    echo ""
    echo "  --- Piper ---"
    run_test "4.7  Piper left channel" 8 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" OMNIVOX_AUDIO_TARGET=left <<'EOF'
tts_say {Left ear only. Left ear only.}
EOF

    run_test "4.8  Piper right channel" 8 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" OMNIVOX_AUDIO_TARGET=right <<'EOF'
tts_say {Right ear only. Right ear only.}
EOF

    run_test "4.9  Piper both channels" 8 \
        OMNIVOX_ENGINE=piper "OMNIVOX_PIPER_MODEL=$PIPER_MODEL" OMNIVOX_AUDIO_TARGET=both <<'EOF'
tts_say {Both ears. Both ears.}
EOF
else
    echo "  4.7-4.9  Piper channel tests                            SKIPPED"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "=== Results: $pass passed, $fail failed ==="
[ "$fail" -eq 0 ]
