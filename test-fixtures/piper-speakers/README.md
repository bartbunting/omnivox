# Deterministic Piper speaker fixtures

These two tiny ONNX graphs are original test data, with no trained voice
weights. Regenerate them with `python3 tools/generate_piper_speaker_fixture.py`.
They accept Piper's native inputs and return 128 samples whose value depends
on the speaker index: alpha returns `0.125 + 0.125 * sid`; beta returns
`0.375 + 0.125 * sid`. They use the text phonemizer and need no eSpeak assets.

Native adapter tests distinguish model and speaker selection through actual
PCM, verify same-model reuse, and require old-model destruction before another
can load. This establishes argument routing and lifecycle behavior, not speech
quality or audible identity of a trained multi-speaker voice. Do not play these
constant samples as a speech demonstration.
