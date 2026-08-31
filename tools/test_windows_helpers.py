#!/usr/bin/env python3
"""Fast source-contract checks for the Windows capture helpers."""

from __future__ import annotations

from pathlib import Path
import unittest


REPOSITORY = Path(__file__).resolve().parent.parent
HELPERS = REPOSITORY / "windows-helpers"


def source(relative: str) -> str:
    return (HELPERS / relative).read_text(encoding="utf-8")


class WindowsHelperSourceTests(unittest.TestCase):
    def test_sources_retain_their_gpl_exception(self) -> None:
        self.assertTrue((HELPERS / "COPYING").is_file())
        for relative in (
            "common/OmnivoxHelperHost.cs",
            "eloquence/OmnivoxEloquenceCapture.cs",
            "eloquence/OmnivoxEloquenceHelper.cs",
            "dectalk/OmnivoxDectalkCapture.cs",
            "dectalk/OmnivoxDectalkHelper.cs",
        ):
            with self.subTest(source=relative):
                contents = source(relative)
                self.assertIn("Copyright (C) 2026 Bart Bunting", contents)
                self.assertIn("SPDX-License-Identifier: GPL-2.0-or-later", contents)

    def test_adapters_advertise_native_text_repertoires(self) -> None:
        self.assertIn(
            'TextRepertoire = "windows_1252"',
            source("eloquence/OmnivoxEloquenceHelper.cs"),
        )
        self.assertIn(
            'TextRepertoire = "iso_8859_1"',
            source("dectalk/OmnivoxDectalkHelper.cs"),
        )

    def test_host_transports_repertoire_and_rejects_bad_unicode(self) -> None:
        host = source("common/OmnivoxHelperHost.cs")
        self.assertIn('capabilities["text_repertoire"]', host)
        self.assertIn("IsWellFormedUnicode(value)", host)

    def test_native_encoders_never_use_replacement_fallback(self) -> None:
        for relative in (
            "eloquence/OmnivoxEloquenceCapture.cs",
            "dectalk/OmnivoxDectalkCapture.cs",
        ):
            with self.subTest(source=relative):
                capture = source(relative)
                self.assertIn("EncoderFallback.ExceptionFallback", capture)
                self.assertNotIn("EncoderFallback.ReplacementFallback", capture)

    def test_build_keeps_helpers_separate_and_32_bit(self) -> None:
        build = source("build.ps1")
        self.assertEqual(build.count("/platform:x86"), 2)
        for expected in (
            "OmnivoxEloquenceHelper32.exe",
            "OmnivoxDectalkHelper32.exe",
            "eloquence\\OmnivoxEloquenceCapture.cs",
            "dectalk\\OmnivoxDectalkCapture.cs",
            'Join-Path $Common "OmnivoxHelperHost.cs"',
        ):
            with self.subTest(expected=expected):
                self.assertIn(expected, build)


if __name__ == "__main__":
    unittest.main()
