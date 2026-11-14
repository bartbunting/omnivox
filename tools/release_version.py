#!/usr/bin/env python3
"""Classify supported releases consistently for packaging and publication."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import re
import subprocess


@dataclass(frozen=True)
class ReleaseVersion:
    version: str
    prerelease: bool

    @property
    def debian_version(self) -> str:
        return self.version.replace("-pre.", "~pre.")

    @property
    def deb_asset(self) -> str:
        # GitHub normalizes '~' in asset names; Debian ordering belongs to the
        # control Version field, independently of the downloaded filename.
        return f"omnivox_{self.version}-1_amd64.deb"


def parse_version(version: str) -> ReleaseVersion:
    number = r"(?:0|[1-9][0-9]*)"
    if not re.fullmatch(rf"{number}\.{number}\.{number}(?:-pre\.[1-9][0-9]*)?", version):
        raise ValueError("expected MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-pre.N (N >= 1)")
    return ReleaseVersion(version, "-pre." in version)


def publish(version: str, repository: str) -> None:
    release = parse_version(version)
    tag = f"v{version}"
    info = json.loads(subprocess.check_output([
        "gh", "release", "view", tag, "--repo", repository,
        "--json", "tagName,isDraft,isPrerelease",
    ], text=True))
    if (info["tagName"] != tag or not info["isDraft"]
            or info["isPrerelease"] != release.prerelease):
        raise ValueError("expected the exact draft with the correct release classification")
    command = [
        "gh", "release", "edit", tag, "--repo", repository, "--verify-tag",
        "--draft=false", f"--prerelease={str(release.prerelease).lower()}",
    ]
    if release.prerelease:
        command.append("--latest=false")
    subprocess.run(command, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--publish", metavar="OWNER/REPO")
    arguments = parser.parse_args()
    try:
        release = parse_version(arguments.version)
        if arguments.github_output:
            with arguments.github_output.open("a") as output:
                output.write(f"version={release.version}\n"
                             f"prerelease={str(release.prerelease).lower()}\n"
                             f"deb_asset={release.deb_asset}\n")
        if arguments.publish:
            publish(release.version, arguments.publish)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
