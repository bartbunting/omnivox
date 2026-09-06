#!/usr/bin/env python3
"""Verify a local .deb in a disposable Ubuntu container, never on the host."""

from __future__ import annotations

import argparse
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

from verify_release import require, verify_checksum, verify_execution


def run(*command: str) -> str:
    return subprocess.check_output(command, text=True).strip()


def check_archive(archive: Path) -> None:
    require(run("dpkg-deb", "-f", str(archive), "Package") == "omnivox",
            "wrong Debian package name")
    require(run("dpkg-deb", "-f", str(archive), "Architecture") == "amd64",
            "wrong Debian architecture")
    data = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(archive)])
    allowed = ("usr/lib/omnivox/", "usr/share/omnivox/", "usr/share/doc/omnivox/",
               "usr/share/emacs/site-lisp/omnivox/")
    with tarfile.open(fileobj=io.BytesIO(data)) as bundle:
        names = set()
        for member in bundle:
            name = member.name.removeprefix("./")
            require(name not in names, f"duplicate package entry: {name}")
            names.add(name)
            require(member.uid == member.gid == 0, f"non-root package owner: {name}")
            if not member.issym():
                require(not member.mode & 0o7022, f"unsafe package permissions: {name}")
            require(member.isdir() or member.isfile() or member.issym(),
                    f"special package entry: {name}")
            require(not name.startswith("/") and ".." not in Path(name).parts,
                    f"unsafe package path: {name}")
            if not member.isdir():
                require(name == "usr/bin/omnivox" or name.startswith(allowed),
                        f"unexpected package payload: {name}")
            if member.issym():
                require(not member.linkname.startswith("/"), f"absolute link: {name}")
        require("usr/lib/omnivox/omnivox" in names, "missing executable")
        require("usr/share/omnivox/espeak-ng-data/phontab" in names, "missing eSpeak data")
        for engine in ("flite", "rutts", "piper", "tgspeechbox"):
            require(not any(f"/{engine}/" in name for name in names),
                    f"optional companion bundled: {engine}")


def speech_check(version: str) -> None:
    require(run("omnivox", "--version") == f"omnivox {version}", "command link failed")
    with tempfile.TemporaryDirectory(prefix="omnivox speech test ") as temporary:
        verify_execution(Path("/usr/bin/omnivox"), version, ["espeak"],
                         Path(temporary), "linux")
    print("Unprivileged voice discovery and non-silent WAV synthesis passed")


def check_release_identity(info: dict, binary_version: str,
                           version: str | None, commit: str | None) -> None:
    if version is not None:
        require(binary_version == version, "binary does not match requested release")
        require(info["package_version"] == f"{version}-1", "wrong release Debian version")
        require(info["distribution"] == "tagged release candidate", "development package in release")
        require(info["corresponding_source"] == f"omnivox-{version}-piper-source.tar.gz",
                "wrong corresponding-source identity")
    if commit is not None:
        require(info["commit"] == commit, "package was built from a different commit")


def installed_check(archive: Path, version: str | None, commit: str | None) -> None:
    require(Path("/.dockerenv").is_file(), "installation tests require a container")
    check_archive(archive)
    verification = run("dpkg", "--verify", "omnivox")
    require(not verification, f"installed file checksum mismatch:\n{verification}")
    info = json.loads(Path("/usr/share/doc/omnivox/BUILD-INFO.json").read_text())
    binary_version = run("omnivox", "--version").removeprefix("omnivox ")
    check_release_identity(info, binary_version, version, commit)
    require(run("dpkg-query", "-W", "-f=${Version}", "omnivox") == info["package_version"],
            "installed package version does not match build identity")
    for link in Path("/usr/lib/omnivox").rglob("*"):
        if link.is_symlink():
            require(link.exists(), f"broken installed link: {link}")
    require(not Path("/usr/share/espeak-ng-data").exists(), "unexpected system eSpeak data")
    subprocess.run(["runuser", "-u", "nobody", "--", "python3", __file__,
                    "--speech-only", binary_version], check=True)
    # Reinstall the exact package, then upgrade using a synthetic metadata-only
    # revision. This fixture is kept inside the disposable container.
    subprocess.run(["apt-get", "install", "-y", "--reinstall", str(archive)], check=True)
    sentinel = Path("/home/omnivox-test/.config/omnivox/preserve-me")
    sentinel.parent.mkdir(parents=True)
    sentinel.write_text("user settings\n")
    with tempfile.TemporaryDirectory(prefix="omnivox-upgrade-") as temporary:
        work = Path(temporary)
        work.chmod(0o755)
        root = work / "root"
        subprocess.run(["dpkg-deb", "--raw-extract", str(archive), str(root)], check=True)
        control = root / "DEBIAN/control"
        old = info["package_version"]
        new = old + "+upgrade1"
        control.write_text(control.read_text().replace(f"Version: {old}\n", f"Version: {new}\n"))
        upgrade = work / "upgrade-test.deb"
        subprocess.run(["dpkg-deb", "--root-owner-group", "--build", str(root), str(upgrade)],
                       check=True)
        subprocess.run(["apt-get", "install", "-y", str(upgrade)], check=True)
        require(run("dpkg-query", "-W", "-f=${Version}", "omnivox") == new,
                "package upgrade did not take effect")
        subprocess.run(["runuser", "-u", "nobody", "--", "python3", __file__,
                        "--speech-only", binary_version], check=True)
    subprocess.run(["apt-get", "purge", "-y", "omnivox"], check=True)
    for path in ("/usr/bin/omnivox", "/usr/lib/omnivox", "/usr/share/omnivox",
                 "/usr/share/doc/omnivox", "/usr/share/emacs/site-lisp/omnivox"):
        require(not Path(path).exists() and not Path(path).is_symlink(),
                f"package file left after purge: {path}")
    require(sentinel.read_text() == "user settings\n", "user configuration changed")
    print("PASS: installation, checksums, speech, reinstall, upgrade, purge, configuration preservation")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", nargs="?", type=Path)
    parser.add_argument("--image", default="ubuntu:26.04")
    parser.add_argument("--checksums", type=Path, help="release sha256sums.txt or local sidecar")
    parser.add_argument("--version", help="expected release version (requires VERSION-1 packaging)")
    parser.add_argument("--commit", help="expected source commit")
    parser.add_argument("--installed", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--speech-only", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.speech_only:
        speech_check(arguments.speech_only)
        return
    if arguments.archive is None:
        parser.error("archive is required")
    archive = arguments.archive.resolve()
    if arguments.installed:
        installed_check(archive, arguments.version, arguments.commit)
        return
    verify_checksum(archive, arguments.checksums or archive.with_suffix(".deb.sha256"))
    check_archive(archive)
    expected = []
    if arguments.version is not None:
        require(archive.name == f"omnivox_{arguments.version}-1_amd64.deb",
                "unexpected release package filename")
        expected.extend(["--version", arguments.version])
    if arguments.commit is not None:
        expected.extend(["--commit", arguments.commit])
    tools = Path(__file__).resolve().parent
    subprocess.run([
        "docker", "run", "--rm", "--platform", "linux/amd64",
        "--mount", f"type=bind,source={archive},target=/candidate.deb,readonly",
        "--mount", f"type=bind,source={tools},target=/tools,readonly",
        "-e", "DEBIAN_FRONTEND=noninteractive", "-e", "PYTHONDONTWRITEBYTECODE=1",
        arguments.image, "sh", "-eu", "-c",
        "printf '%s\n' 'path-include=/usr/share/doc/omnivox' "
        "'path-include=/usr/share/doc/omnivox/*' > /etc/dpkg/dpkg.cfg.d/zz-omnivox-test "
        "&& apt-get update && apt-get install -y python3 /candidate.deb "
        '&& python3 /tools/test_deb_install.py --installed /candidate.deb "$@"',
        "omnivox-deb-test", *expected,
    ], check=True)


if __name__ == "__main__":
    main()
