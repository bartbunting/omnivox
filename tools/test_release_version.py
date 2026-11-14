#!/usr/bin/env python3
"""Verify prerelease classification, asset names and publication safeguards."""

import json
import unittest
from unittest.mock import patch

from release_version import parse_version, publish
from verify_release_asset_set import expected_asset_names, require_exact_names, AssetSetError


class ReleaseVersionTests(unittest.TestCase):
    def test_stable_and_numbered_prerelease(self):
        self.assertFalse(parse_version("1.19.0").prerelease)
        pre = parse_version("1.19.0-pre.12")
        self.assertTrue(pre.prerelease)
        self.assertEqual(pre.debian_version, "1.19.0~pre.12")
        self.assertEqual(pre.deb_asset, "omnivox_1.19.0-pre.12-1_amd64.deb")
        for value in ("v1.19.0", "1.19.0-pre.0", "1.19.0-pre.01", "01.19.0",
                      "1.19.0-rc.1", "1.19.0+dev", "1.19.0\n", "../1.19.0"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse_version(value)

    def test_prerelease_requires_full_asset_set_and_matching_versions(self):
        version = "1.19.0-pre.1"
        names = expected_asset_names(version)
        self.assertEqual(len(names), len(expected_asset_names("1.19.0")))
        self.assertIn("omnivox_1.19.0-pre.1-1_amd64.deb", names)
        self.assertIn("omnivox-1.19.0-pre.1-macos-arm64.tar.gz", names)
        require_exact_names(list(names), version)
        names.remove("omnivox_1.19.0-pre.1-1_amd64.deb")
        for incorrect in ("omnivox_1.19.0~pre.1-1_amd64.deb",
                          "omnivox_1.19.0.pre.1-1_amd64.deb"):
            with self.subTest(filename=incorrect), self.assertRaises(AssetSetError):
                require_exact_names([*names, incorrect], version)

    def test_publication_keeps_prerelease_out_of_latest(self):
        for version, pre in (("1.19.0", False), ("1.19.0-pre.1", True)):
            info = dict(tagName=f"v{version}", isDraft=True, isPrerelease=pre)
            with patch("release_version.subprocess.check_output", return_value=json.dumps(info)), \
                    patch("release_version.subprocess.run") as run:
                publish(version, "owner/repo")
            command = run.call_args.args[0]
            self.assertIn(f"--prerelease={str(pre).lower()}", command)
            if pre:
                self.assertIn("--latest=false", command)
            else:
                self.assertFalse(any(flag.startswith("--latest") for flag in command))
            self.assertIn("--verify-tag", command)

    def test_wrong_tag_classification_or_published_release_cannot_publish(self):
        info = dict(tagName="v1.19.0-pre.1", isDraft=True, isPrerelease=True)
        for key, value in (("tagName", "v1.18.0"), ("isDraft", False),
                           ("isPrerelease", False)):
            with self.subTest(key=key), \
                    patch("release_version.subprocess.check_output",
                          return_value=json.dumps({**info, key: value})), \
                    patch("release_version.subprocess.run") as run:
                with self.assertRaises(ValueError):
                    publish("1.19.0-pre.1", "owner/repo")
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
