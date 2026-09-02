#!/usr/bin/env python3
"""Regression tests for the exact release asset-set gate."""

from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import verify_release_asset_set as assets


class ReleaseAssetSetTests(unittest.TestCase):
    def test_expected_set_has_24_archives_and_one_manifest(self) -> None:
        names = assets.expected_asset_names("1.6.4")
        self.assertEqual(len(names), 25)
        self.assertIn("sha256sums.txt", names)
        self.assertIn("omnivox-1.6.4-windows-arm64.zip", names)
        self.assertIn("omnivox-1.6.4-flite-linux-arm64.tar.gz", names)
        self.assertIn("omnivox-1.6.4-rutts-source.tar.gz", names)
        self.assertIn("omnivox-1.6.4-piper-macos-x64.tar.gz", names)
        self.assertNotIn("omnivox-1.6.4-piper-windows-arm64.zip", names)

    def test_names_gate_rejects_stale_and_missing_assets(self) -> None:
        names = sorted(assets.expected_asset_names("1.6.4"))
        names.remove("omnivox-1.6.4-flite-linux-x64.tar.gz")
        names.append("omnivox-1.6.3-flite-linux-x64.tar.gz")
        with self.assertRaisesRegex(
            assets.AssetSetError, "missing assets.*unexpected assets"
        ):
            assets.require_exact_names(names, "1.6.4")

    def test_names_gate_rejects_duplicates(self) -> None:
        names = sorted(assets.expected_asset_names("1.6.4"))
        names.append(names[0])
        with self.assertRaisesRegex(assets.AssetSetError, "duplicate assets"):
            assets.require_exact_names(names, "1.6.4")

    def test_directory_gate_requires_exact_checksums(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            names = assets.expected_asset_names("1.6.4")
            archives = sorted(names - {"sha256sums.txt"})
            for name in archives:
                (directory / name).write_bytes(b"archive")
            (directory / "sha256sums.txt").write_text(
                "\n".join(f"{'0' * 64}  {name}" for name in archives) + "\n",
                encoding="utf-8",
            )

            assets.verify_directory(directory, "1.6.4")

            (directory / "sha256sums.txt").write_text(
                f"{'0' * 64}  {archives[0]}\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(assets.AssetSetError, "manifest mismatch"):
                assets.verify_directory(directory, "1.6.4")


if __name__ == "__main__":
    unittest.main()
