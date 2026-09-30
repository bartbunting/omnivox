#!/usr/bin/env python3
"""Regression tests for the generic release payload contract."""

from __future__ import annotations

from pathlib import Path
import shutil
import struct
import tempfile
import unittest

import verify_release


class ReleaseLayoutTests(unittest.TestCase):
    def write_pe(self, path: Path, machine: int) -> None:
        """Write the inspected portion of a PE executable for MACHINE."""
        content = bytearray(512)
        content[:2] = b"MZ"
        struct.pack_into("<I", content, 0x3C, 0x80)
        content[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<H", content, 0x84, machine)
        path.write_bytes(content)

    def make_payload(
        self, root: Path, version: str, *, windows_helpers: bool
    ) -> None:
        """Create a minimal generic Windows payload below ROOT."""
        self.write_pe(root / "omnivox.exe", 0x8664)
        (root / "omnivox-voices.el").write_text("; adapter\n" * 20)
        (root / "LICENSE").write_text("project license\n" * 20)
        (root / "LICENSING.md").write_text("component map\n" * 20)

        data = root / "espeak-ng-data"
        data.mkdir()
        (data / "phontab").write_text("table")
        for index in range(99):
            (data / f"voice-{index}").write_text("data")

        notices = root / "third-party-licenses"
        notices.mkdir()
        for name in (
            "THIRD-PARTY-NOTICES.md",
            "eSpeak-NG-GPL-3.0.txt",
            "Unicode-Data-License.txt",
            "NetBSD-getopt.c",
        ):
            (notices / name).write_text("notice")
        (notices / "omnivox-Cargo.lock").write_text(
            f'[[package]]\nname = "omnivox-cli"\nversion = "{version}"\n'
        )

        rhvoice = root / "rhvoice"
        rhvoice.mkdir()
        self.write_pe(rhvoice / "omnivox-rhvoice-helper.exe", 0x8664)

        if not windows_helpers:
            return
        for name in (
            "OmnivoxDectalkHelper32.exe",
            "OmnivoxEloquenceHelper32.exe",
        ):
            self.write_pe(root / name, 0x14C)
        (root / "WINDOWS-HELPERS-COPYING").write_text("GPL\n" * 4_000)
        source = root / "windows-helpers-source"
        helpers = Path(__file__).resolve().parent.parent / "windows-helpers"
        for name in (
            "COPYING",
            "Makefile",
            "README.md",
            "build.ps1",
        ):
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(helpers / name, path)
        # Match the release staging recipe using real sources. Duplicating the
        # verifier's file list here hid newly added build inputs from this test.
        for directory in ("common", "dectalk", "eloquence"):
            shutil.copytree(helpers / directory, source / directory)

    def test_windows_current_requires_runtime_helpers_and_packaged_source(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.make_payload(root, "1.12.0", windows_helpers=True)
            verify_release.verify_layout(root, "windows", "1.12.0")

            (root / "OmnivoxEloquenceHelper32.exe").unlink()
            with self.assertRaisesRegex(
                verify_release.VerificationError,
                "unexpected archive root entries",
            ):
                verify_release.verify_layout(root, "windows", "1.12.0")

    def test_windows_source_requires_both_parameter_implementations(self) -> None:
        for relative in (
            "dectalk/OmnivoxDectalkParameters.cs",
            "eloquence/OmnivoxEloquenceParameters.cs",
        ):
            with self.subTest(source=relative), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.make_payload(root, "1.12.0", windows_helpers=True)
                (root / "windows-helpers-source" / relative).unlink()
                with self.assertRaisesRegex(
                    verify_release.VerificationError, "missing \\[.*Parameters"
                ):
                    verify_release.verify_layout(root, "windows", "1.12.0")

    def test_windows_source_rejects_unexpected_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.make_payload(root, "1.12.0", windows_helpers=True)
            (root / "windows-helpers-source" / "runtime.dll").write_bytes(b"unexpected")
            with self.assertRaisesRegex(
                verify_release.VerificationError, "unexpected \\['runtime.dll'\\]"
            ):
                verify_release.verify_layout(root, "windows", "1.12.0")

    def test_windows_170_historical_layout_remains_valid(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.make_payload(root, "1.7.0", windows_helpers=False)
            verify_release.verify_layout(root, "windows", "1.7.0")

    def test_windows_runtime_helpers_must_be_x86(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.make_payload(root, "1.7.1", windows_helpers=True)
            self.write_pe(root / "OmnivoxDectalkHelper32.exe", 0x8664)
            with self.assertRaisesRegex(
                verify_release.VerificationError,
                "architecture mismatch",
            ):
                verify_release.verify_layout(root, "windows", "1.7.1")


if __name__ == "__main__":
    unittest.main()
