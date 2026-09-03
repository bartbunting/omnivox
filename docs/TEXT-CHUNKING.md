# Text Chunking in Omnivox

## Current behavior

Omnivox prepares speech text and then divides long text into bounded synthesis
chunks. The hard limit is 15 whitespace-delimited words. Within each 15-word
window it prefers the latest useful boundary in this order:

1. sentence terminator or line break;
2. clause punctuation; or
3. the hard word limit.

Common Unicode sentence terminators and closing quotation/bracket characters
are recognized. Whitespace between chunks is not synthesized. Short input is
retained as one chunk without copying its spelling or internal whitespace.

The implementation is `chunk_prepared_speech()` in
`omnivox-cli/src/text.rs`. Queue, immediate, preview, and structured-timeline
paths use it through `omnivox-cli/src/pipeline.rs`.

## Why it exists

Interactive speech benefits from placing the first bounded result into the
audio pipeline without requiring one very long native synthesis. Small chunks
also bound per-call work and make cancellation checks and fallback opportunities
more frequent.

Chunking does **not** promise that a native engine invokes exactly one internal
buffer callback. Engine adapters validate their native output and either relay
bounded progressive windows or collect one complete result according to their
advertised capability. The useful contract is that Omnivox makes one structured
synthesis request per prepared chunk and can begin queueing supported output
before synthesizing later chunks.

Every chunk is independently canonicalized, silence-trimmed, processed, and
scheduled. Leading/trailing padding is rate-aware to avoid clipping. The final
effect or overlay tail is emitted only after the final timeline window.

## Source mapping

`PreparedSpeechChunk` records its UTF-8 byte range in the complete prepared
text. Caller-requested structured timeline offsets are first remapped through
punctuation expansion and CamelCase splitting, then assigned to one chunk with
their declared before/after affinity. The chunker also filters and rebases any
internal capitalization anchors supplied in a prepared value. Current text
preparation deliberately infers no such anchors: semantic clients carry
capitalization actions explicitly, and ordinary speech preserves letter case.

This is why callers must not re-split a prepared string independently: doing so
would detach requested anchors from the text sent to the engine.

## Trade-offs

- Smaller synthesis calls improve first-result latency and cancellation
  opportunities.
- Sentence/clause preference reduces arbitrary prosody breaks compared with a
  fixed 15-word split.
- Multiple calls add engine setup overhead and can expose a boundary in engines
  with markedly different per-utterance prosody.
- A hard limit is intentionally retained so a long punctuation-free line
  cannot become an unbounded synthesis call.

The limit is not currently configurable. A public option should be added only
if matched real-engine benchmarks show a useful cross-platform trade-off.

## Verification

Unit tests cover short/exact/long input, sentence and clause preference,
newlines, Unicode closers, hard-limit fallback, prepared-anchor rebasing, and
structured action offsets across chunks. Run the repository's locked workspace
suite:

```sh
cargo test --locked --workspace
```

Manual latency evaluation must separate protocol admission, synthesis result,
audio queueing, mixer-source consumption, and physical audible onset. A log
showing that chunks were synthesized is not by itself an audible-onset
measurement.
