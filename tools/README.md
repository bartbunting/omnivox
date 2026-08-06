# Developer Tools

## Windows helper session stress

`stress_helper.py` keeps one protocol-v2 helper process alive across repeated
synthesis calls. It validates negotiation, descriptor identity, realized voice,
audio sequence and frame totals, word/sentence marker bounds, periodic pings,
and clean shutdown.

From WSL, after building the Emacsvox helpers:

```sh
python3 tools/stress_helper.py \
  ~/src/emacsvox/servers/windows-eloquence/bin/OmnivoxEloquenceHelper32.exe \
  --engine-id eloquence --iterations 100

python3 tools/stress_helper.py \
  ~/src/emacsvox/servers/windows-dectalk/bin/OmnivoxDectalkHelper32.exe \
  --engine-id dectalk --iterations 100
```

Use `--voice-id` to select a non-default voice and repeat `--helper-arg` when a
helper needs an explicit native DLL argument. The RSS value available from WSL
belongs to its interop launcher; use native Windows process tooling for a
working-set growth measurement.

The in-process eSpeak counterpart is ignored during ordinary unit tests and
can be run explicitly:

```sh
cargo test --locked -p omnivox-tts stress_repeated_synthesis_session \
  -- --ignored --nocapture
```

## WAV comparison

`compare_wavs.py` compares two PCM16 or float32 WAV files after silence
trimming and reports RMS difference, correlation, SNR, and per-segment values.
`tts_reference.swift` captures a macOS AVSpeechSynthesizer reference WAV.
