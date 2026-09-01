#!/usr/bin/env python3
"""Tests for audit_speech_rates.py."""

from __future__ import annotations

from pathlib import Path
import struct
import sys
import tempfile
import unittest

import audit_speech_rates


FAKE_OMNIVOX = r'''
from pathlib import Path
import struct
import sys

if "--version" in sys.argv:
    print("omnivox fake")
    raise SystemExit(0)

rate = float(sys.argv[sys.argv.index("--rate") + 1])
dump = sys.argv.index("--dump-wav")
output = Path(sys.argv[dump + 2])
raw = Path(str(output).replace(".wav", "_raw.wav"))
samples = max(1, int(16000 / max(rate, 0.01)))
payload = b"\0\0\0\0" * samples
header = (
    b"RIFF" + struct.pack("<I", 36 + len(payload)) + b"WAVE"
    + b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, 16000, 64000, 4, 32)
    + b"data" + struct.pack("<I", len(payload))
)
raw.write_bytes(header + payload)
output.write_bytes(header + payload)
'''


class AuditSpeechRatesTests(unittest.TestCase):
    def test_target_parser_preserves_helper_voice_identifier(self) -> None:
        target = audit_speech_rates.parse_target("rhvoice=rhvoice:Alan")
        self.assertEqual(target.engine, "rhvoice")
        self.assertEqual(target.voice, "rhvoice:Alan")

    def test_float_wav_duration_uses_data_bytes_and_byte_rate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            wav = Path(directory) / "sample.wav"
            payload = b"\0\0\0\0" * 8000
            junk = b"x"
            body = (
                b"JUNK" + struct.pack("<I", len(junk)) + junk + b"\0"
                + b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, 16000, 64000, 4, 32)
                + b"data" + struct.pack("<I", len(payload)) + payload
            )
            wav.write_bytes(b"RIFF" + struct.pack("<I", 4 + len(body)) + b"WAVE" + body)
            self.assertEqual(audit_speech_rates.wav_duration(wav), 0.5)

    def test_fake_engine_records_faster_duration_at_higher_rate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "fake_omnivox.py"
            fake.write_text(FAKE_OMNIVOX, encoding="utf-8")
            result = audit_speech_rates.run_target(
                [sys.executable, str(fake)],
                audit_speech_rates.EngineTarget("fake", "voice:id"),
                [0.4, 0.8],
                2,
                "one two three four",
                4,
                root,
                5.0,
                None,
                False,
            )
            summaries = result["summary"]
            self.assertGreater(
                summaries[0]["median_pipeline_duration_seconds"],
                summaries[1]["median_pipeline_duration_seconds"],
            )
            self.assertLess(
                summaries[0]["median_words_per_minute"],
                summaries[1]["median_words_per_minute"],
            )
            with self.assertRaises(audit_speech_rates.AuditError):
                audit_speech_rates.run_target(
                    [sys.executable, str(fake)],
                    audit_speech_rates.EngineTarget("fake", "voice:id"),
                    [0.4, 0.8],
                    2,
                    "one two three four",
                    4,
                    root,
                    5.0,
                    None,
                    False,
                )

    def test_report_writer_refuses_to_replace_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "audit.json"
            audit_speech_rates.write_report(report, {"schema_version": 1})
            with self.assertRaises(audit_speech_rates.AuditError):
                audit_speech_rates.write_report(report, {"schema_version": 2})


if __name__ == "__main__":
    unittest.main()
