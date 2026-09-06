#!/usr/bin/env python3
"""Build a native development or guarded tagged-release Debian package."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib


REPOSITORY = Path(__file__).resolve().parent.parent
MAINTAINER = "Bart Bunting <bartbunting@users.noreply.github.com>"
NATIVE_TARGET = "x86_64-unknown-linux-gnu"


def output(*command: str, cwd: Path = REPOSITORY) -> str:
    return subprocess.check_output(command, cwd=cwd, text=True).strip()


def require_release_tree(version: str) -> None:
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise RuntimeError("release packaging requires a stable MAJOR.MINOR.PATCH version")
    if output("git", "status", "--porcelain", "--untracked-files=all"):
        raise RuntimeError("release packaging requires a clean source tree")
    tag = f"v{version}"
    if output("git", "rev-parse", f"refs/tags/{tag}^{{commit}}") != output(
            "git", "rev-parse", "HEAD"):
        raise RuntimeError(f"release packaging requires HEAD at {tag}")
    ci_ref = os.environ.get("GITHUB_REF")
    if ci_ref and ci_ref != f"refs/tags/{tag}":
        raise RuntimeError(f"release packaging requires the CI ref refs/tags/{tag}")


def debian_version(version: str, identity: dict[str, str], release: bool,
                   revision: str | None) -> str:
    revision = revision or ("1" if release else "0local1")
    if not re.fullmatch(r"[0-9][A-Za-z0-9.+~]*", revision):
        raise RuntimeError("invalid Debian revision")
    if release:
        if revision != "1":
            raise RuntimeError("the release asset contract requires Debian revision 1")
        return f"{version}-1"
    date = output("git", "show", "-s", "--format=%cs", "HEAD").replace("-", "")
    return (f"{version}+git{date}.{identity['commit'][:7]}."
            f"{identity['source_sha256'][:12]}-{revision}")


def source_identity() -> dict[str, str]:
    """Include uncommitted and untracked source, without touching ignored builds."""
    names = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=REPOSITORY,
    ).split(b"\0")
    digest = hashlib.sha256()
    for name in sorted(set(names) - {b""}):
        path = REPOSITORY / os.fsdecode(name)
        digest.update(name + b"\0")
        if path.is_symlink():
            digest.update(b"symlink\0" + os.fsencode(os.readlink(path)))
        elif path.is_file():
            digest.update(str(path.stat().st_mode & 0o777).encode() + b"\0")
            digest.update(hashlib.sha256(path.read_bytes()).digest())
        else:
            digest.update(b"deleted\0")
    return {
        "commit": output("git", "rev-parse", "HEAD"),
        "source_sha256": digest.hexdigest(),
        "commit_epoch": output("git", "show", "-s", "--format=%ct", "HEAD"),
    }


def copy_file(source: Path, destination: Path, executable: bool = False) -> None:
    if source.is_symlink() or not source.is_file():
        raise RuntimeError(f"required regular payload file is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def copy_tree(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise RuntimeError(f"required payload directory is missing: {source}")
    for path in sorted(source.rglob("*")):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise RuntimeError(f"unsupported payload entry: {path}")
        if path.is_file():
            copy_file(path, destination / path.relative_to(source))


def check_elf(binary: Path) -> None:
    with binary.open("rb") as stream:
        header = stream.read(20)
    if (len(header) != 20 or header[:6] != b"\x7fELF\x02\x01"
            or struct.unpack_from("<H", header, 18)[0] != 62):
        raise RuntimeError(f"expected an amd64 Linux ELF executable: {binary}")


def stage(profile: Path, root: Path) -> list[Path]:
    runtime = root / "usr/lib/omnivox"
    shared = root / "usr/share/omnivox"
    docs = root / "usr/share/doc/omnivox"
    binaries = []
    for name in ("omnivox", "rhvoice/omnivox-rhvoice-helper"):
        source = profile / name
        check_elf(source)
        destination = runtime / name
        copy_file(source, destination, executable=True)
        subprocess.run(["strip", "--strip-unneeded", str(destination)], check=True)
        binaries.append(destination)
    copy_tree(profile / "espeak-ng-data", shared / "espeak-ng-data")
    if not (shared / "espeak-ng-data/phontab").is_file():
        raise RuntimeError("packaged eSpeak data has no phontab")
    copy_tree(profile / "third-party-licenses", docs / "third-party-licenses")
    for name in ("eSpeak-NG-GPL-3.0.txt", "Unicode-Data-License.txt",
                 "NetBSD-getopt.c", "omnivox-Cargo.lock", "THIRD-PARTY-NOTICES.md"):
        if not (docs / "third-party-licenses" / name).is_file():
            raise RuntimeError(f"missing required notice: {name}")
    for name in ("LICENSE", "LICENSING.md"):
        copy_file(profile / name, docs / name)
    copy_file(REPOSITORY / "elisp/omnivox-voices.el",
              root / "usr/share/emacs/site-lisp/omnivox/omnivox-voices.el")
    copy_file(REPOSITORY / "windows-helpers/COPYING", docs / "GPL-2")
    copy_file(REPOSITORY / "docs/DEBIAN.md", docs / "README.Debian")
    (docs / "copyright").write_text(
        "Omnivox-authored source: MIT; see LICENSE.\n"
        "The main executable incorporates eSpeak NG: GPL-3.0-or-later.\n"
        "See LICENSING.md and third-party-licenses for component terms and notices.\n"
        "omnivox-voices.el retains its GPL-2.0-or-later notice; see GPL-2.\n\n"
        + (docs / "LICENSE").read_text(), encoding="utf-8",
    )
    links = {
        root / "usr/bin/omnivox": "../lib/omnivox/omnivox",
        runtime / "espeak-ng-data": "../../share/omnivox/espeak-ng-data",
        runtime / "third-party-licenses": "../../share/doc/omnivox/third-party-licenses",
        runtime / "LICENSE": "../../share/doc/omnivox/LICENSE",
        runtime / "LICENSING.md": "../../share/doc/omnivox/LICENSING.md",
    }
    for link, target in links.items():
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(target)
    return binaries


def package(arguments: argparse.Namespace) -> Path:
    if sys.platform != "linux" or output("dpkg", "--print-architecture") != "amd64":
        raise RuntimeError("initial Debian packaging supports native Linux amd64 only")
    for tool in ("make", "cargo", "strip", "dpkg-shlibdeps", "dpkg-deb"):
        if shutil.which(tool) is None:
            raise RuntimeError(f"required build tool is missing: {tool}")
    with (REPOSITORY / "Cargo.toml").open("rb") as stream:
        version = tomllib.load(stream)["workspace"]["package"]["version"]
    if arguments.release:
        require_release_tree(version)
    identity = source_identity()
    package_version = debian_version(version, identity, arguments.release, arguments.revision)
    # Override any developer cross-build default, and read only this build's
    # explicit target directory rather than a potentially stale native binary.
    build_environment = {**os.environ, "CARGO_BUILD_TARGET": NATIVE_TARGET}
    subprocess.run(["make", "build"], cwd=REPOSITORY, env=build_environment, check=True)
    if identity != source_identity():
        raise RuntimeError("source changed during the build; rerun packaging")
    metadata = json.loads(output("cargo", "metadata", "--locked", "--no-deps",
                                 "--format-version", "1"))
    profile = Path(metadata["target_directory"]) / NATIVE_TARGET / "release"
    if output(str(profile / "omnivox"), "--version") != f"omnivox {version}":
        raise RuntimeError("built binary version does not match Cargo.toml")
    destination = arguments.output_dir.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f"omnivox_{package_version}_amd64.deb"
    environment = os.environ.copy()
    environment["SOURCE_DATE_EPOCH"] = identity["commit_epoch"]
    with tempfile.TemporaryDirectory(prefix=".package-deb-", dir=destination) as temporary:
        work = Path(temporary)
        root = work / "debian/omnivox"
        binaries = stage(profile, root)
        control = root / "DEBIAN"
        control.mkdir()
        # dpkg-shlibdeps needs a source control file even for a binary-only build.
        (work / "debian").mkdir(exist_ok=True)
        (work / "debian/control").write_text(
            f"Source: omnivox\nMaintainer: {MAINTAINER}\n\n"
            "Package: omnivox\nArchitecture: amd64\nDescription: speech server\n"
        )
        dependencies = output("dpkg-shlibdeps", "-O",
                              *(str(p.relative_to(work)) for p in binaries), cwd=work)
        prefix = "shlibs:Depends="
        if not dependencies.startswith(prefix) or "\n" in dependencies:
            raise RuntimeError(f"unexpected shared-library dependencies: {dependencies}")
        provenance = {
            **identity, "package_version": package_version,
            "rustc": output("rustc", "--version"),
            "build_os": Path("/etc/os-release").read_text(),
            "distribution": ("tagged release candidate" if arguments.release else
                             "local development candidate; not a published release"),
        }
        if arguments.release:
            provenance["corresponding_source"] = f"omnivox-{version}-piper-source.tar.gz"
        (root / "usr/share/doc/omnivox/BUILD-INFO.json").write_text(
            json.dumps(provenance, indent=2, sort_keys=True) + "\n"
        )
        files = sorted(p for p in root.rglob("*") if p.is_file() and not p.is_symlink())
        size = sum((p.stat().st_size + 1023) // 1024 for p in files)
        (control / "control").write_text(
            f"Package: omnivox\nVersion: {package_version}\nArchitecture: amd64\n"
            f"Maintainer: {MAINTAINER}\nSection: sound\nPriority: optional\n"
            f"Installed-Size: {size}\nDepends: {dependencies[len(prefix):]}\n"
            "Homepage: https://github.com/bartbunting/omnivox\n"
            "Description: speech server for Emacs with bundled eSpeak NG\n"
            " Provides speech synthesis, audio mixing, and optional engine helpers.\n"
            " Includes matching eSpeak NG data and the RHVoice integration helper.\n"
            " RHVoice libraries and voices are supplied separately by the user.\n"
            + (" This package was built from a verified release tag.\n" if arguments.release
               else " This package is a local development build.\n")
        )
        (control / "md5sums").write_text("".join(
            f"{hashlib.md5(p.read_bytes()).hexdigest()}  {p.relative_to(root)}\n"
            for p in files
        ))
        for path in [root, *root.rglob("*")]:
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() or path in binaries else 0o644)
            os.utime(path, (int(identity["commit_epoch"]),) * 2, follow_symlinks=False)
        candidate = work / archive.name
        subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build",
                        str(root), str(candidate)], env=environment, check=True)
        candidate.replace(archive)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".deb.sha256").write_text(f"{checksum}  {archive.name}\n")
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=REPOSITORY / "target/debian")
    parser.add_argument("--release", action="store_true",
                        help="require a clean matching tag and produce VERSION-1")
    parser.add_argument("--revision", help="Debian revision (local: 0local1; release: 1)")
    arguments = parser.parse_args()
    try:
        print(package(arguments))
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
