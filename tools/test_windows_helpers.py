#!/usr/bin/env python3
"""Fast source-contract checks for the Windows capture helpers."""

from __future__ import annotations

from pathlib import Path
import re
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
            "common/OmnivoxNativeLibrary.cs",
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

    def test_host_omits_empty_synthesis_stream_frames(self) -> None:
        host = source("common/OmnivoxHelperHost.cs")
        self.assertIn(
            "for (int offset = 0; offset < audio.Length; offset += maximumChunk)",
            host,
        )
        self.assertIn("if (markers.Length > 0)", host)

    def test_windows_helpers_offer_bounded_progressive_pcm_in_v5(self) -> None:
        host = source("common/OmnivoxHelperHost.cs")
        self.assertIn("LatestProtocolVersion = 5", host)
        self.assertIn('"streaming_pcm" : "buffered_pcm"', host)
        self.assertIn("CanonicalSampleRate = 44100", host)
        self.assertIn("CanonicalChannels = 2", host)
        self.assertIn("MaximumAudioChunkBytes", host)
        for relative in (
            "eloquence/OmnivoxEloquenceHelper.cs",
            "dectalk/OmnivoxDectalkHelper.cs",
        ):
            with self.subTest(source=relative):
                self.assertIn(
                    "SupportsProgressiveSynthesis { get { return true; } }",
                    source(relative),
                )

    def test_dectalk_holds_one_native_block_for_late_markers(self) -> None:
        capture = source("dectalk/OmnivoxDectalkCapture.cs")
        marker_write = capture.index("sink.Markers(markerBatch)")
        audio_write = capture.index("sink.Audio(readyAudio")
        self.assertLess(marker_write, audio_write)
        self.assertIn("readyAudio = pendingProgressiveAudio", capture)
        self.assertIn("pendingProgressiveAudio = audio", capture)

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
        self.assertEqual(
            build.count('Join-Path $Common "OmnivoxNativeLibrary.cs"'), 2
        )
        for expected in (
            "OmnivoxEloquenceHelper32.exe",
            "OmnivoxDectalkHelper32.exe",
            "eloquence\\OmnivoxEloquenceCapture.cs",
            "dectalk\\OmnivoxDectalkCapture.cs",
            'Join-Path $Common "OmnivoxHelperHost.cs"',
        ):
            with self.subTest(expected=expected):
                self.assertIn(expected, build)

    def test_native_loading_is_absolute_restricted_and_validated(self) -> None:
        loader = source("common/OmnivoxNativeLibrary.cs")
        for expected in (
            "Path.IsPathRooted(path)",
            'EntryPoint = "LoadLibraryExW"',
            "LoadLibrarySearchDllLoadDir | LoadLibrarySearchSystem32",
            "ValidateX86PortableExecutable(FullPath, displayName)",
            "ImageFileMachineI386",
            "GetProcAddress(module, export)",
        ):
            with self.subTest(expected=expected):
                self.assertIn(expected, loader)

        combined_captures = source(
            "eloquence/OmnivoxEloquenceCapture.cs"
        ) + source("dectalk/OmnivoxDectalkCapture.cs")
        self.assertNotIn("SetDllDirectory", combined_captures)
        self.assertNotIn("Environment.CurrentDirectory", combined_captures)
        self.assertNotIn("LoadLibrary(", combined_captures)

    def test_all_engine_exports_are_resolved_before_use(self) -> None:
        eloquence = source("eloquence/OmnivoxEloquenceCapture.cs")
        for export in (
            "eciVersion",
            "eciNewEx",
            "eciDelete",
            "eciStop",
            "eciClearInput",
            "eciSynthesize",
            "eciSynchronize",
            "eciAddText",
            "eciInsertIndex",
            "eciSetParam",
            "eciRegisterCallback",
            "eciSetOutputBuffer",
        ):
            with self.subTest(engine="eloquence", export=export):
                self.assertRegex(
                    eloquence,
                    r"library\.Resolve<[^>]+>\(\s*\"" +
                    re.escape(export) + r"\"\)",
                )

        dectalk = source("dectalk/OmnivoxDectalkCapture.cs")
        for export in (
            "TextToSpeechStartupExFonix",
            "TextToSpeechShutdown",
            "TextToSpeechSpeak",
            "TextToSpeechReset",
            "TextToSpeechSync",
            "TextToSpeechSetRate",
            "TextToSpeechOpenInMemory",
            "TextToSpeechCloseInMemory",
            "TextToSpeechAddBuffer",
            "TextToSpeechVersion",
        ):
            with self.subTest(engine="dectalk", export=export):
                self.assertRegex(
                    dectalk,
                    r"library\.Resolve<[^>]+>\(\s*\"" +
                    re.escape(export) + r"\"\)",
                )
        self.assertIn(
            "uint versionCode = native.TextToSpeechVersion(out versionValue)",
            dectalk,
        )
        self.assertIn("versionCode == 0", dectalk)
        self.assertNotIn("Check(native.TextToSpeechVersion", dectalk)

    def test_missing_runtime_keeps_protocol_loop_available(self) -> None:
        host = source("common/OmnivoxHelperHost.cs")
        self.assertIn('OmnivoxHelperLog.Event("runtime_unavailable"', host)
        self.assertIn('throw Fault("not_available", runtimeUnavailableReason', host)
        self.assertIn('WriteError(requestId, "not_available"', host)
        for relative in (
            "eloquence/OmnivoxEloquenceHelper.cs",
            "dectalk/OmnivoxDectalkHelper.cs",
        ):
            with self.subTest(source=relative):
                self.assertIn("OmnivoxHelperRuntime.Run", source(relative))


if __name__ == "__main__":
    unittest.main()
