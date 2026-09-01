#!/usr/bin/env python3
"""Build and atomically stage the source-built RuTTS companion."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


RUTTS_VERSION = "6.3.3"
RUTTS_COMMIT = "2848d2892097320ed37fc963b439b15803f47f0c"
SUPPORTED_TARGETS = {
    "aarch64-apple-darwin": ("macos-arm64", "omnivox-rutts-helper"),
    "aarch64-pc-windows-msvc": ("windows-arm64", "omnivox-rutts-helper.exe"),
    "aarch64-unknown-linux-gnu": ("linux-arm64", "omnivox-rutts-helper"),
    "x86_64-apple-darwin": ("macos-x64", "omnivox-rutts-helper"),
    "x86_64-pc-windows-gnu": ("windows-x64-gnu", "omnivox-rutts-helper.exe"),
    "x86_64-pc-windows-msvc": ("windows-x64", "omnivox-rutts-helper.exe"),
    "x86_64-unknown-linux-gnu": ("linux-x64", "omnivox-rutts-helper"),
}


class StagingError(RuntimeError):
    """A Cargo output cannot form a verified RuTTS companion directory."""


def usage() -> str:
    return (
        "usage: python tools/build_rutts.py [cargo build arguments]\n\n"
        "Prepares locked RuTTS v6.3.3 source, builds omnivox-rutts-helper "
        "with locked Cargo dependencies, and stages a self-contained rutts/ "
        "directory.\n\n"
        "examples:\n"
        "  python3 tools/build_rutts.py --release\n"
        "  python3 tools/build_rutts.py --release --target aarch64-unknown-linux-gnu"
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_output(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def host_target() -> str:
    output = subprocess.run(
        ["rustc", "-vV"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    ).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise StagingError("rustc did not report its host target")


def requested_target(arguments: list[str]) -> str:
    values: list[str] = []
    iterator = iter(range(len(arguments)))
    for index in iterator:
        argument = arguments[index]
        if argument == "--target":
            if index + 1 >= len(arguments):
                raise StagingError("--target requires a target triple")
            values.append(arguments[index + 1])
            next(iterator, None)
        elif argument.startswith("--target="):
            values.append(argument.split("=", 1)[1])
    if len(set(values)) > 1:
        raise StagingError(f"conflicting Cargo targets: {values}")
    return values[0] if values else host_target()


def prepare_source(repository: Path) -> tuple[Path, dict[str, object]]:
    command = [sys.executable, str(repository / "tools/prepare_rutts_inputs.py")]
    if configured := os.environ.get("OMNIVOX_RUTTS_INPUTS_DIR"):
        command.extend(["--output", configured])
    print("+ " + " ".join(command), file=sys.stderr)
    subprocess.run(command, check=True)
    output = (
        Path(configured).resolve()
        if configured
        else repository / "target/rutts-inputs" / RUTTS_VERSION
    )
    marker = json.loads((output / "PREPARED.json").read_text(encoding="utf-8"))
    source = Path(str(marker["source_path"]))
    if not source.is_dir():
        raise StagingError(f"prepared RuTTS source is missing: {source}")
    return source, marker


def render_cargo_message(message: dict[str, object]) -> None:
    if message.get("reason") != "compiler-message":
        return
    compiler_message = message.get("message")
    if isinstance(compiler_message, dict):
        rendered = compiler_message.get("rendered")
        if isinstance(rendered, str):
            sys.stderr.write(rendered)


def build(arguments: list[str], source: Path) -> Path:
    forbidden = ("--message-format", "--package")
    for argument in arguments:
        if argument == "-p" or argument.startswith(forbidden):
            raise StagingError(
                "tools/build_rutts.py owns Cargo's package and message-format selection"
            )
    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "--package",
        "omnivox-rutts-helper",
        *arguments,
    ]
    environment = dict(os.environ)
    environment["OMNIVOX_RUTTS_SOURCE_DIR"] = str(source)
    print("+ " + " ".join(command), file=sys.stderr)
    process = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if process.stdout is None:
        raise StagingError("failed to capture Cargo build messages")
    executables: set[Path] = set()
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        render_cargo_message(message)
        target = message.get("target")
        executable = message.get("executable")
        if (
            message.get("reason") == "compiler-artifact"
            and isinstance(target, dict)
            and target.get("name") == "omnivox-rutts-helper"
            and isinstance(executable, str)
        ):
            executables.add(Path(executable).resolve())
    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    if len(executables) != 1:
        rendered = "\n  ".join(str(path) for path in sorted(executables)) or "<none>"
        raise StagingError(
            f"Cargo reported {len(executables)} RuTTS helper executables; "
            f"refusing to guess:\n  {rendered}"
        )
    executable = executables.pop()
    if not executable.is_file():
        raise StagingError(f"Cargo-reported helper is missing: {executable}")
    return executable


def checksum_manifest(directory: Path) -> str:
    lines = []
    for path in sorted(item for item in directory.rglob("*") if item.is_file()):
        if path.name == "SHA256SUMS":
            continue
        relative = path.relative_to(directory).as_posix()
        lines.append(f"{sha256_file(path)}  {relative}")
    return "\n".join(lines) + "\n"


def companion_readme(target: str) -> str:
    return f"""# Omnivox RuTTS companion

This `{target}` companion contains the Omnivox helper and RuTTS v{RUTTS_VERSION},
statically built from commit `{RUTTS_COMMIT}`. It provides the built-in male
and female Russian voices.

Place this top-level `rutts/` directory beside `omnivox` or `omnivox.exe`.
RuLex is not included or loaded by this companion.

`LICENSE` covers Omnivox-authored code. `third-party-licenses/RuTTS-LICENSE.txt`
is RuTTS's complete upstream MIT licence. `SOURCE-PROVENANCE.json` records the
exact source input used for this executable.
"""


def stage(
    repository: Path,
    executable: Path,
    target: str,
    source: Path,
    marker: dict[str, object],
) -> Path:
    if target not in SUPPORTED_TARGETS:
        raise StagingError(f"unsupported RuTTS companion target: {target}")
    suffix, expected_helper = SUPPORTED_TARGETS[target]
    if executable.name != expected_helper:
        raise StagingError(
            f"unexpected helper name for {target}: {executable.name}"
        )
    profile = executable.parent
    destination = profile / "rutts"
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{destination.name}.tmp-{os.getpid()}-", dir=profile)
    )
    try:
        shutil.copy2(executable, temporary / executable.name)
        shutil.copy2(repository / "LICENSE", temporary / "LICENSE")
        shutil.copy2(repository / "docs/LICENSING.md", temporary / "LICENSING.md")
        shutil.copy2(
            repository / "omnivox-rutts-sys/source-inputs.json",
            temporary / "source-inputs.json",
        )
        notices = temporary / "third-party-licenses"
        notices.mkdir()
        shutil.copy2(source / "LICENSE", notices / "RuTTS-LICENSE.txt")
        shutil.copy2(repository / "Cargo.lock", notices / "omnivox-Cargo.lock")
        (temporary / "README.md").write_text(companion_readme(target), encoding="utf-8")
        provenance = {
            "schema_version": 1,
            "artifact": f"omnivox-rutts-companion-{suffix}",
            "target": target,
            "built_in_voices": ["male", "female"],
            "rulex_included": False,
            "rutts": {
                "version": RUTTS_VERSION,
                "commit": RUTTS_COMMIT,
                "archive_sha256": marker["archive_sha256"],
                "source_tree_sha256": marker["source_tree_sha256"],
                "verified_before_build": True,
            },
            "source_input_lock_sha256": marker["lock_file_sha256"],
            "omnivox": {
                "commit": git_output(repository, "rev-parse", "HEAD"),
                "tracked_worktree_dirty": bool(
                    git_output(repository, "status", "--porcelain", "--untracked-files=no")
                ),
            },
        }
        (temporary / "SOURCE-PROVENANCE.json").write_text(
            json.dumps(provenance, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        (temporary / "SHA256SUMS").write_text(
            checksum_manifest(temporary), encoding="utf-8"
        )
        if destination.exists():
            if not destination.is_dir():
                raise StagingError(
                    f"RuTTS companion destination is not a directory: {destination}"
                )
            shutil.rmtree(destination)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    print(f"Staged RuTTS companion in {destination}", file=sys.stderr)
    return destination


def main() -> int:
    if any(argument in {"-h", "--help"} for argument in sys.argv[1:]):
        print(usage())
        return 0
    repository = Path(__file__).resolve().parent.parent
    try:
        arguments = sys.argv[1:]
        target = requested_target(arguments)
        source, marker = prepare_source(repository)
        stage(repository, build(arguments, source), target, source, marker)
    except (
        OSError,
        StagingError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
