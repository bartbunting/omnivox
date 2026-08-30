# Historical Text-Chunking Implementation Note

This filename is retained for old links. The original implementation report
described an early fixed-whitespace splitter in `omnivox-cli/src/main.rs`, with
line numbers and test counts that no longer match the repository.

The current implementation is sentence/clause-aware, preserves UTF-8 source
offsets for timeline actions, and lives in `omnivox-cli/src/text.rs` with its
call sites in `omnivox-cli/src/pipeline.rs`.

See [TEXT-CHUNKING.md](../TEXT-CHUNKING.md) for the maintained behavior,
rationale, integration points, limitations, and validation command. Use Git
history when the retired implementation details are needed.
