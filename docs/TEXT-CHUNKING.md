# Text Chunking in Omnivox

## Overview

Text chunking splits long text into smaller segments (approximately 15 words each) before TTS synthesis. This enables aggressive silence trimming and ensures predictable audio buffer behavior.

## Rationale

### Problem: Multi-buffer Utterances

When AVSpeechSynthesizer (macOS) or SpeechSynthesizer (Windows) processes long text, it may:

- Generate multiple audio buffers per utterance
- Add unpredictable pauses between internal phrase boundaries
- Produce variable-length leading/trailing silence

This makes aggressive silence trimming risky — you might accidentally remove silence that's *between* words in a multi-buffer utterance, causing word loss.

### Solution: Single-buffer Utterances

By splitting text into ~15 word chunks, each chunk:

- Generates exactly one audio buffer from the TTS engine
- Has predictable silence only at start/end (not mid-utterance)
- Can be aggressively trimmed without risk of cutting words

This was implemented in swiftmac (commit d567621) to enable fast, responsive speech with minimal latency.

## Implementation Details

### Chunk Size

15 words is the target chunk size, chosen empirically:

- Small enough to ensure single-buffer synthesis
- Large enough to maintain natural prosody within chunks
- Avoids chunking overhead for short utterances

### Chunking Algorithm

```
Split text on whitespace
If word count <= 15: return as single chunk
Otherwise:
  - Group into 15-word segments
  - Final segment may be < 15 words
```

### Sequential Playback

Chunks are synthesized and queued sequentially:

1. Chunk 1 → TTS → audio buffer → pipeline → speech stream
2. Chunk 2 → TTS → audio buffer → pipeline → speech stream
3. ...

Each chunk flows through the pipeline independently. The silence trimmer can aggressively cut silence from each chunk because we know there are no mid-utterance boundaries.

### Silence Trimming

With chunking, the `SilenceTrimmer` effect can use aggressive thresholds:

- Trim leading silence down to near-zero
- Trim trailing silence down to near-zero
- No risk of cutting inter-word pauses (they don't exist within a chunk)

This produces extremely responsive speech output with minimal delay between text arrival and audio playback.

## Terminology

**Chunk**: A segment of text (typically ~15 words) that is synthesized as a single utterance, producing a single audio buffer. Also called a "text chunk" or "utterance chunk."

**Single-buffer utterance**: A TTS synthesis operation that produces exactly one audio buffer callback, with no internal phrase boundaries.

**Multi-buffer utterance**: A TTS synthesis operation that produces multiple audio buffer callbacks for a single piece of text, typically with internal pauses.

## Integration Points

### In omnivox-cli/src/main.rs

Chunking should be applied in these locations:

1. **`CommandId::TtsSay`** (immediate speech, line ~728)
   - After `preprocess_text()`
   - Before `engine.synthesize()`

2. **`process_queue_items()` → `QueueItem::Speech`** (line ~917)
   - After `preprocess_text()`
   - Before `engine.synthesize()`

3. **Letter speaking** (line ~680) - NO chunking
   - Single character, already tiny
   - Skip chunking for single-word utterances

4. **Version announcement** (line ~768, ~565) - NO chunking
   - Short text, unlikely to exceed 15 words

### Pseudo-code

```rust
fn chunk_text(text: &str, max_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();

    if words.len() <= max_words {
        return vec![text.to_string()];
    }

    words
        .chunks(max_words)
        .map(|chunk| chunk.join(" "))
        .collect()
}

// Usage in TtsSay handler:
let processed_text = preprocess_text(&text, state);
for chunk in chunk_text(&processed_text, 15) {
    if let Ok(tts_buf) = engine.synthesize(&chunk, &settings) {
        let mut buf = tts_buffer_to_audio_buffer(tts_buf);
        let pipeline = build_speech_pipeline(state);
        let _ = pipeline.process(&mut buf);
        let _ = streams.queue(StreamType::Speech, &buf);
    }
}
```

## Benefits

- **Lower latency**: First chunk plays immediately, remaining chunks follow
- **Aggressive trimming**: Safe to remove all inter-chunk silence
- **Predictable buffering**: Each chunk = one buffer, simplifies audio pipeline
- **Memory efficiency**: Process and discard chunks sequentially

## Trade-offs

- **Prosody boundaries**: Natural intonation may break at 15-word boundaries
- **Performance**: N synthesis calls vs. 1 for long text (negligible for typical use)
- **Complexity**: Adds loop logic to synthesis callsites

In practice, the latency and responsiveness benefits far outweigh these costs for interactive screen reader use.

## Testing

Manual test for chunking behavior:

```bash
# Long text (> 15 words) should chunk and play seamlessly
echo 'tts_say {This is a very long sentence with many words that should be split into multiple chunks to ensure single buffer utterances and enable aggressive silence trimming for maximum responsiveness.}' | ./target/release/omnivox

# Short text (< 15 words) should not chunk
echo 'tts_say {Short sentence here.}' | ./target/release/omnivox
```

Look for debug logs indicating chunking (future enhancement).

## Future Work

- Make chunk size configurable via `--chunk-size` CLI flag?
- Smarter chunking on sentence/clause boundaries instead of word count?
- Benchmark synthesis overhead for typical Emacspeak workloads
