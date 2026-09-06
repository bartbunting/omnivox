#!/usr/bin/env python3
"""Regression checks for Debian release identity and publication safeguards."""

from __future__ import annotations

import os
import subprocess
import unittest
from unittest.mock import patch

import package_deb as package
from test_deb_install import check_release_identity
from verify_release import VerificationError


class ReleaseTreeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.environment = patch.dict(os.environ, {}, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_accepts_clean_matching_tag_without_changing_git(self) -> None:
        with patch.object(package, "output", side_effect=["", "abc", "abc"]) as git:
            package.require_release_tree("1.8.0")
        self.assertEqual(git.call_args_list[1].args,
                         ("git", "rev-parse", "refs/tags/v1.8.0^{commit}"))

    def test_rejects_tracked_and_untracked_changes_before_tag_lookup(self) -> None:
        for status in (" M Cargo.toml", "?? new-source.rs"):
            with self.subTest(status=status), patch.object(package, "output", return_value=status) as git:
                with self.assertRaisesRegex(RuntimeError, "clean source tree"):
                    package.require_release_tree("1.8.0")
                self.assertEqual(git.call_count, 1)

    def test_rejects_wrong_commit(self) -> None:
        with patch.object(package, "output", side_effect=["", "old", "new"]):
            with self.assertRaisesRegex(RuntimeError, "HEAD at v1.8.0"):
                package.require_release_tree("1.8.0")

    def test_missing_tag_is_fatal(self) -> None:
        with patch.object(package, "output", side_effect=["", subprocess.CalledProcessError(128, "git")]):
            with self.assertRaises(subprocess.CalledProcessError):
                package.require_release_tree("1.8.0")

    def test_rejects_wrong_ci_ref(self) -> None:
        for ref in ("refs/heads/main", "refs/tags/v1.7.1"):
            with self.subTest(ref=ref), patch.dict(os.environ, {"GITHUB_REF": ref}):
                with patch.object(package, "output", side_effect=["", "abc", "abc"]):
                    with self.assertRaisesRegex(RuntimeError, "CI ref"):
                        package.require_release_tree("1.8.0")

    def test_accepts_matching_ci_ref(self) -> None:
        with patch.dict(os.environ, {"GITHUB_REF": "refs/tags/v1.8.0"}):
            with patch.object(package, "output", side_effect=["", "abc", "abc"]):
                package.require_release_tree("1.8.0")

    def test_rejects_nonstable_version(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "stable"):
            package.require_release_tree("1.8.0-rc1")


class PackageIdentityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.source = {"commit": "a" * 40, "source_sha256": "b" * 64}
        self.release = {
            "commit": "a" * 40, "package_version": "1.8.0-1",
            "distribution": "tagged release candidate",
            "corresponding_source": "omnivox-1.8.0-piper-source.tar.gz",
        }

    def test_release_version_uses_exact_asset_contract(self) -> None:
        self.assertEqual(package.debian_version("1.8.0", self.source, True, None), "1.8.0-1")
        with self.assertRaisesRegex(RuntimeError, "revision 1"):
            package.debian_version("1.8.0", self.source, True, "0local1")

    def test_development_version_retains_source_identity(self) -> None:
        with patch.object(package, "output", return_value="2026-09-06"):
            version = package.debian_version("1.8.0", self.source, False, None)
        self.assertEqual(version, "1.8.0+git20260906.aaaaaaa.bbbbbbbbbbbb-0local1")

    def test_revision_cannot_inject_control_fields(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "invalid Debian revision"):
            package.debian_version("1.8.0", self.source, False, "1\nDepends: evil")

    def test_downloaded_release_identity_matches_binary_and_tag(self) -> None:
        check_release_identity(self.release, "1.8.0", "1.8.0", "a" * 40)

    def test_downloaded_release_rejects_wrong_binary(self) -> None:
        with self.assertRaisesRegex(VerificationError, "binary"):
            check_release_identity(self.release, "1.7.1", "1.8.0", "a" * 40)

    def test_downloaded_release_rejects_wrong_metadata(self) -> None:
        for field, value in (
            ("commit", "c" * 40),
            ("package_version", "1.8.0+gitabcdef0-0local1"),
            ("distribution", "local development candidate; not a published release"),
            ("corresponding_source", "omnivox-1.7.1-piper-source.tar.gz"),
        ):
            with self.subTest(field=field), self.assertRaises(VerificationError):
                check_release_identity({**self.release, field: value}, "1.8.0", "1.8.0", "a" * 40)


if __name__ == "__main__":
    unittest.main()
