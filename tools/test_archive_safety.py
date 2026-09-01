#!/usr/bin/env python3
"""Regression tests for bounded safe archive extraction."""

from __future__ import annotations

from io import BytesIO
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

import prepare_piper_inputs
import verify_release


def write_tar(path: Path, entries: dict[str, bytes]) -> None:
    with tarfile.open(path, "w:gz") as archive:
        for name, contents in entries.items():
            member = tarfile.TarInfo(name)
            member.size = len(contents)
            archive.addfile(member, BytesIO(contents))


def write_zip(path: Path, entries: dict[str, bytes]) -> None:
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, contents in entries.items():
            archive.writestr(name, contents)


class ReleaseArchiveSafetyTests(unittest.TestCase):
    def test_tar_rejects_member_and_uncompressed_size_overflow(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "release.tar.gz"
            write_tar(archive, {"one": b"1", "two": b"2"})

            with mock.patch.object(verify_release, "MAX_ARCHIVE_MEMBERS", 1):
                with self.assertRaisesRegex(
                    verify_release.VerificationError, "member limit"
                ):
                    verify_release.extract_tar(archive, root / "members")

            with mock.patch.object(
                verify_release, "MAX_ARCHIVE_UNCOMPRESSED_BYTES", 1
            ):
                with self.assertRaisesRegex(
                    verify_release.VerificationError, "uncompressed limit"
                ):
                    verify_release.extract_tar(archive, root / "bytes")

    def test_zip_rejects_member_and_uncompressed_size_overflow(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "release.zip"
            write_zip(archive, {"one": b"1", "two": b"2"})

            with mock.patch.object(verify_release, "MAX_ARCHIVE_MEMBERS", 1):
                with self.assertRaisesRegex(
                    verify_release.VerificationError, "member limit"
                ):
                    verify_release.extract_zip(archive, root / "members")

            with mock.patch.object(
                verify_release, "MAX_ARCHIVE_UNCOMPRESSED_BYTES", 1
            ):
                with self.assertRaisesRegex(
                    verify_release.VerificationError, "uncompressed limit"
                ):
                    verify_release.extract_zip(archive, root / "bytes")


class InputArchiveSafetyTests(unittest.TestCase):
    def test_locked_input_tar_rejects_member_overflow(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "input.tar.gz"
            write_tar(archive, {"source/one": b"1", "source/two": b"2"})

            with mock.patch.object(
                prepare_piper_inputs, "MAX_INPUT_ARCHIVE_MEMBERS", 1
            ):
                with self.assertRaisesRegex(
                    prepare_piper_inputs.PreparationError, "member limit"
                ):
                    prepare_piper_inputs.extract_tar_archive(
                        archive, root / "members", "source", False
                    )

    def test_locked_input_zip_rejects_uncompressed_size_overflow(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "input.zip"
            write_zip(archive, {"source/one": b"12"})

            with mock.patch.object(
                prepare_piper_inputs, "MAX_INPUT_ARCHIVE_UNCOMPRESSED_BYTES", 1
            ):
                with self.assertRaisesRegex(
                    prepare_piper_inputs.PreparationError, "uncompressed limit"
                ):
                    prepare_piper_inputs.extract_zip_archive(
                        archive, root / "bytes", "source"
                    )


if __name__ == "__main__":
    unittest.main()
