#!/usr/bin/env python3
"""Check TGSpeechBox archive portability, integrity and release guards."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import package_tgspeechbox as package
import verify_release as common
import verify_tgspeechbox_release as verify


class ArchiveTests(unittest.TestCase):
    def test_archives_preserve_helper_permissions_and_detect_changed_data(self):
        for extension, writer, platform in (
            ("tar.gz", package.write_tar, "linux"),
            ("zip", package.write_zip, "windows"),
        ):
            with self.subTest(extension=extension), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / "source"
                source.mkdir()
                helper = source / "helper"
                helper.write_bytes(b"test executable")
                helper.chmod(0o755)
                (source / "packs").mkdir()
                data = source / "packs/phonemes.yaml"
                data.write_text("test data\n")
                (source / "SHA256SUMS").write_text(
                    f"{package.sha256_file(helper)}  helper\n"
                    f"{package.sha256_file(data)}  packs/phonemes.yaml\n"
                )
                first = root / f"first.{extension}"
                second = root / f"second.{extension}"
                writer(source, first, 1_700_000_000)
                writer(source, second, 1_700_000_000)
                self.assertEqual(first.read_bytes(), second.read_bytes())
                extracted = root / "Relocated payload with spaces"
                extracted.mkdir()
                common.extract_archive(first, extracted, platform)
                payload = extracted / "tgspeechbox"
                verify.verify_inner_checksums(payload)
                if platform == "linux":
                    self.assertTrue((payload / "helper").stat().st_mode & 0o111)
                (payload / "packs/phonemes.yaml").write_text("changed")
                with self.assertRaisesRegex(common.VerificationError, "inner checksums"):
                    verify.verify_inner_checksums(payload)

    def test_archive_names_cover_native_targets_and_preserve_gnu_name(self):
        cases = {
            "aarch64-unknown-linux-gnu": ("linux-arm64", "tar.gz"),
            "x86_64-apple-darwin": ("macos-x64", "tar.gz"),
            "aarch64-apple-darwin": ("macos-arm64", "tar.gz"),
            "aarch64-pc-windows-msvc": ("windows-arm64", "zip"),
            "x86_64-pc-windows-gnu": ("windows-x64", "zip"),
            "x86_64-pc-windows-msvc": ("windows-x64", "zip"),
        }
        for target, expected in cases.items():
            self.assertEqual(package.archive_identity(target), expected)
        with self.assertRaises(package.PackagingError):
            package.archive_identity("unsupported")

    def test_explicit_target_and_version_select_matching_paths(self):
        with patch("sys.argv", ["package_tgspeechbox.py", "--version", "9.0.0",
                                "--target", "aarch64-apple-darwin"]):
            repository = Path(__file__).resolve().parent.parent
            args = package.parse_arguments(repository)
        self.assertEqual(args.output.name, "omnivox-9.0.0-tgspeechbox-macos-arm64.tar.gz")
        self.assertEqual(args.staged, repository / "target/aarch64-apple-darwin/release/tgspeechbox")


if __name__ == "__main__":
    unittest.main()
