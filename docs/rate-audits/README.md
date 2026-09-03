# Retained TGSpeechBox Rate Audit

These reports support the TGSpeechBox calibration accepted in ADR 0007. All
three use the standard 22-word English corpus from `tools/audit_speech_rates.py`
and canonical post-pipeline WAV duration.

- `2026-09-03-tgspeechbox-before.json` is the one-repetition baseline from the
  provisional exponential mapping at pinned TGSpeechBox `v-310@f5ec247`.
- `2026-09-03-eloquence-v1-reference.json` is the matching one-repetition
  Windows Eloquence `v1` target curve.
- `2026-09-03-tgspeechbox-after.json` is the three-repetition acceptance from
  the calibrated Adam `en-us` table at the default 44.1 kHz native rate.

The baseline and acceptance covered host rates
`0,0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1,1.2,1.5,2`. The acceptance command
was:

```sh
python3 tools/audit_speech_rates.py target/release/omnivox \
  --target tgspeechbox=en-us/adam \
  --rates 0,0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1,1.2,1.5,2 \
  --repetitions 3 \
  --json-output target/rate-audit-tgspeechbox-after.json
```

The calibrated result differs from Eloquence by at most 0.05% through host
rate `1.0`. At `1.2`, TGSpeechBox reaches its measured `4x` ceiling of 690.944
WPM and remains there through host rate `2.0`.
