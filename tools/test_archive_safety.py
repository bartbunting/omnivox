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

import package_piper_source
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
    def test_rejects_windows_paths_on_every_host(self) -> None:
        for name in (
            "C:../outside.txt", "D:/outside.txt", "source/C:../outside.txt",
            "source/file:stream", "source/.. /outside.txt", "source/file.",
            "../outside.txt", "/outside.txt", "source\\outside.txt",
        ):
            for validator in (
                verify_release.safe_parts,
                prepare_piper_inputs.safe_parts,
                package_piper_source.safe_parts,
            ):
                with self.subTest(name=name, validator=validator.__module__):
                    with self.assertRaises(RuntimeError):
                        validator(name)

    def test_normal_names_still_extract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for extension, write, extract in (
                ("zip", write_zip, verify_release.extract_zip),
                ("tar.gz", write_tar, verify_release.extract_tar),
            ):
                archive = root / f"release.{extension}"
                destination = root / extension
                write(archive, {"source/speech data.txt": b"data"})
                extract(archive, destination)
                self.assertEqual(
                    (destination / "source/speech data.txt").read_bytes(), b"data"
                )

    def test_existing_links_cannot_redirect_extraction(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            outside = root / "outside"
            outside.mkdir()
            (outside / "file").write_bytes(b"keep")
            for extension, write, extract in (
                ("zip", write_zip, verify_release.extract_zip),
                ("tar.gz", write_tar, verify_release.extract_tar),
            ):
                destination = root / extension
                destination.mkdir()
                try:
                    (destination / "link").symlink_to(outside, target_is_directory=True)
                except OSError:
                    self.skipTest("host cannot create symlinks")
                archive = root / f"release.{extension}"
                write(archive, {"link/file": b"overwrite"})
                with self.assertRaisesRegex(RuntimeError, "leaves extraction directory"):
                    extract(archive, destination)
                self.assertEqual((outside / "file").read_bytes(), b"keep")

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
    def test_input_extractors_reject_windows_drive_components(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for extension, write, extract in (
                ("zip", write_zip, prepare_piper_inputs.extract_zip_archive),
                ("tar.gz", write_tar,
                 lambda archive, target, prefix: prepare_piper_inputs.extract_tar_archive(
                     archive, target, prefix, False
                 )),
            ):
                archive = root / f"input.{extension}"
                write(archive, {"source/C:../outside.txt": b"bad"})
                with self.assertRaisesRegex(RuntimeError, "unsafe archive member"):
                    extract(archive, root / extension, "source")
                self.assertFalse((root / extension).exists())

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
