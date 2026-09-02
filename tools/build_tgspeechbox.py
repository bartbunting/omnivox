#!/usr/bin/env python3
"""Build and atomically stage the source-built TGSpeechBox companion."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


RELEASE = "v-310b802"
COMMIT = "7515ae055e45d2d15cae01d7fe081ce951dcd5cd"
ESPEAK_PACKAGE = "#espeak-rs-sys@0.1.9"
PROTOCOL_VERSION = 5
VOICE_INVENTORY_SCHEMA_VERSION = 1
EXPECTED_VOICE_COUNT = 154
VOICE_INVENTORY_FILENAME = "VOICE-INVENTORY.json"
SAMPLE_RATE_ENVIRONMENT_VARIABLE = "OMNIVOX_TGSPEECHBOX_SAMPLE_RATE"
DEFAULT_SAMPLE_RATE = 44_100
SUPPORTED_SAMPLE_RATES = (22_050, 44_100)
VOICE_INVENTORY_FILENAMES = {
    sample_rate: f"VOICE-INVENTORY-{sample_rate}.json"
    for sample_rate in SUPPORTED_SAMPLE_RATES
}
SUPPORTED_TARGETS = {
    "x86_64-pc-windows-gnu": ("windows-x64-gnu", "omnivox-tgspeechbox-helper.exe"),
    "x86_64-pc-windows-msvc": ("windows-x64", "omnivox-tgspeechbox-helper.exe"),
    "x86_64-unknown-linux-gnu": ("linux-x64", "omnivox-tgspeechbox-helper"),
}


class StagingError(RuntimeError):
    """A Cargo output cannot form a TGSpeechBox companion directory."""


def usage() -> str:
    return (
        "usage: python3 tools/build_tgspeechbox.py [cargo build arguments]\n\n"
        "Prepares locked TGSpeechBox source, builds omnivox-tgspeechbox-helper "
        "with locked Cargo dependencies, and stages a self-contained "
        "tgspeechbox/ directory.\n\n"
        "examples:\n"
        "  python3 tools/build_tgspeechbox.py --release\n"
        "  python3 tools/build_tgspeechbox.py --release --target x86_64-pc-windows-gnu"
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
    command = [sys.executable, str(repository / "tools/prepare_tgspeechbox_inputs.py")]
    if configured := os.environ.get("OMNIVOX_TGSPEECHBOX_INPUTS_DIR"):
        command.extend(["--output", configured])
    print("+ " + " ".join(command), file=sys.stderr)
    subprocess.run(command, check=True)
    output = (
        Path(configured).resolve()
        if configured
        else repository / "target/tgspeechbox-inputs" / RELEASE
    )
    marker = json.loads((output / "PREPARED.json").read_text(encoding="utf-8"))
    source = Path(str(marker["source_path"]))
    if not source.is_dir():
        raise StagingError(f"prepared TGSpeechBox source is missing: {source}")
    return source, marker


def render_cargo_message(message: dict[str, object]) -> None:
    if message.get("reason") != "compiler-message":
        return
    compiler_message = message.get("message")
    if isinstance(compiler_message, dict):
        rendered = compiler_message.get("rendered")
        if isinstance(rendered, str):
            sys.stderr.write(rendered)


def collect_host_espeak_output(source: Path, release: bool) -> Path:
    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "--package",
        "omnivox-tgspeechbox-helper",
    ]
    if release:
        command.append("--release")
    environment = dict(os.environ)
    environment["OMNIVOX_TGSPEECHBOX_SOURCE_DIR"] = str(source)
    print(
        "+ " + " ".join(command) + "  # host-generated portable eSpeak data",
        file=sys.stderr,
    )
    process = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if process.stdout is None:
        raise StagingError("failed to capture host Cargo build messages")
    outputs: set[Path] = set()
    for line in process.stdout:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        render_cargo_message(message)
        if (
            message.get("reason") == "build-script-executed"
            and ESPEAK_PACKAGE in str(message.get("package_id", ""))
            and isinstance(message.get("out_dir"), str)
        ):
            outputs.add(Path(str(message["out_dir"])).resolve())
    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    usable = sorted(
        output for output in outputs if (output / "share/espeak-ng-data/phontab").is_file()
    )
    if len(usable) != 1:
        rendered = "\n  ".join(str(path) for path in usable) or "<none>"
        raise StagingError(
            f"host Cargo reported {len(usable)} usable eSpeak-ng outputs; "
            f"refusing to guess:\n  {rendered}"
        )
    return usable[0]


def build(arguments: list[str], source: Path) -> tuple[Path, Path]:
    forbidden = ("--message-format", "--package")
    for argument in arguments:
        if argument == "-p" or argument.startswith(forbidden):
            raise StagingError(
                "tools/build_tgspeechbox.py owns Cargo's package and message-format selection"
            )
    command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json-render-diagnostics",
        "--package",
        "omnivox-tgspeechbox-helper",
        *arguments,
    ]
    environment = dict(os.environ)
    environment["OMNIVOX_TGSPEECHBOX_SOURCE_DIR"] = str(source)
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
    espeak_outputs: set[Path] = set()
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
            and target.get("name") == "omnivox-tgspeechbox-helper"
            and isinstance(executable, str)
        ):
            executables.add(Path(executable).resolve())
        if (
            message.get("reason") == "build-script-executed"
            and ESPEAK_PACKAGE in str(message.get("package_id", ""))
            and isinstance(message.get("out_dir"), str)
        ):
            espeak_outputs.add(Path(str(message["out_dir"])).resolve())
    return_code = process.wait()
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)
    if len(executables) != 1:
        rendered = "\n  ".join(str(path) for path in sorted(executables)) or "<none>"
        raise StagingError(
            f"Cargo reported {len(executables)} TGSpeechBox helper executables; "
            f"refusing to guess:\n  {rendered}"
        )
    usable_espeak = sorted(
        output
        for output in espeak_outputs
        if (output / "share/espeak-ng-data/phontab").is_file()
    )
    if not usable_espeak:
        usable_espeak = [collect_host_espeak_output(source, "--release" in arguments)]
    if len(usable_espeak) != 1:
        rendered = "\n  ".join(str(path) for path in usable_espeak) or "<none>"
        raise StagingError(
            f"Cargo reported {len(usable_espeak)} usable eSpeak-ng outputs; "
            f"refusing to guess:\n  {rendered}"
        )
    executable = executables.pop()
    if not executable.is_file():
        raise StagingError(f"Cargo-reported helper is missing: {executable}")
    return executable, usable_espeak[0]


def checksum_manifest(directory: Path) -> str:
    lines = []
    for path in sorted(item for item in directory.rglob("*") if item.is_file()):
        if path.name == "SHA256SUMS":
            continue
        relative = path.relative_to(directory).as_posix()
        lines.append(f"{sha256_file(path)}  {relative}")
    return "\n".join(lines) + "\n"


def generate_voice_inventory(
    helper: Path, source_identity: object, sample_rate: int
) -> dict[str, object]:
    if (
        not isinstance(source_identity, str)
        or len(source_identity) != 64
        or any(character not in "0123456789abcdef" for character in source_identity)
    ):
        raise StagingError("prepared source marker has an invalid lock-file identity")
    requests = [
        {
            "protocol_version": PROTOCOL_VERSION,
            "request_id": 1,
            "type": "hello",
            "supported_protocol_versions": [5, 4, 3, 2, 1],
        },
        {
            "protocol_version": PROTOCOL_VERSION,
            "request_id": 2,
            "type": "describe",
        },
        {
            "protocol_version": PROTOCOL_VERSION,
            "request_id": 3,
            "type": "shutdown",
        },
    ]
    encoded = "".join(
        json.dumps(request, separators=(",", ":")) + "\n" for request in requests
    )
    environment = dict(os.environ)
    environment[SAMPLE_RATE_ENVIRONMENT_VARIABLE] = str(sample_rate)
    if helper.suffix.lower() == ".exe":
        forwarded = [item for item in environment.get("WSLENV", "").split(":") if item]
        if SAMPLE_RATE_ENVIRONMENT_VARIABLE not in forwarded:
            forwarded.append(SAMPLE_RATE_ENVIRONMENT_VARIABLE)
        environment["WSLENV"] = ":".join(forwarded)
    print(f"+ {helper}  # generate cached voice inventory", file=sys.stderr)
    try:
        result = subprocess.run(
            [str(helper)],
            env=environment,
            input=encoded,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=90,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise StagingError("TGSpeechBox helper timed out while generating inventory") from error
    if result.returncode != 0:
        diagnostics = result.stderr.strip()[-2000:] or "<no diagnostics>"
        raise StagingError(
            f"TGSpeechBox helper exited with {result.returncode} while generating inventory: "
            f"{diagnostics}"
        )
    try:
        responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise StagingError("TGSpeechBox helper returned invalid inventory JSON") from error
    if len(responses) != 3:
        raise StagingError(
            f"TGSpeechBox helper returned {len(responses)} inventory responses, expected 3"
        )
    expected = ((1, "hello"), (2, "descriptor"), (3, "shutting_down"))
    for response, (request_id, response_type) in zip(responses, expected, strict=True):
        if (
            response.get("protocol_version") != PROTOCOL_VERSION
            or response.get("request_id") != request_id
            or response.get("type") != response_type
        ):
            raise StagingError(f"unexpected TGSpeechBox inventory response: {response}")
    descriptor = responses[1].get("descriptor")
    if not isinstance(descriptor, dict) or descriptor.get("id") != "tgspeechbox":
        raise StagingError("TGSpeechBox helper returned the wrong engine descriptor")
    if f"native {sample_rate} Hz" not in str(descriptor.get("version", "")):
        raise StagingError("TGSpeechBox helper descriptor has the wrong native sample rate")
    voices = descriptor.get("voices")
    if not isinstance(voices, list) or len(voices) != EXPECTED_VOICE_COUNT:
        count = len(voices) if isinstance(voices, list) else "invalid"
        raise StagingError(
            f"TGSpeechBox helper returned {count} voices, expected {EXPECTED_VOICE_COUNT}"
        )
    return {
        "schema_version": VOICE_INVENTORY_SCHEMA_VERSION,
        "engine_id": "tgspeechbox",
        "source_identity": source_identity,
        "descriptor": descriptor,
    }


def companion_readme(target: str) -> str:
    return f"""# Omnivox TGSpeechBox companion

This experimental `{target}` companion contains the Omnivox helper,
TGSpeechBox `{RELEASE}` from commit `{COMMIT}`, its YAML language packs, and
the pinned Omnivox eSpeak-ng phonemizer/data.

Place this top-level `tgspeechbox/` directory beside `omnivox` or
`omnivox.exe`. The helper exposes TGSpeechBox profiles through the existing
Omnivox helper protocol. Its rate mapping is provisional and it advertises no
markers until retained calibration and source-offset evidence are available.
Rate-matched voice inventories for native 44.1 and 22.05 kHz operation permit
switching with `OMNIVOX_TGSPEECHBOX_SAMPLE_RATE` and a server restart.

The combined helper is GPLv3 because it links eSpeak-ng. TGSpeechBox itself is
MIT-licensed. Complete notices and the Cargo lock are under
`third-party-licenses/`; `SOURCE-PROVENANCE.json` records the exact inputs.
"""


def stage(
    repository: Path,
    executable: Path,
    espeak_output: Path,
    target: str,
    source: Path,
    marker: dict[str, object],
) -> Path:
    if target not in SUPPORTED_TARGETS:
        raise StagingError(f"unsupported TGSpeechBox companion target: {target}")
    suffix, expected_helper = SUPPORTED_TARGETS[target]
    if executable.name != expected_helper:
        raise StagingError(f"unexpected helper name for {target}: {executable.name}")
    profile = executable.parent
    destination = profile / "tgspeechbox"
    temporary = Path(
        tempfile.mkdtemp(prefix=f".{destination.name}.tmp-{os.getpid()}-", dir=profile)
    )
    try:
        shutil.copy2(executable, temporary / executable.name)
        shutil.copytree(source / "packs", temporary / "packs")
        shutil.copytree(espeak_output / "share/espeak-ng-data", temporary / "espeak-ng-data")
        shutil.copy2(repository / "LICENSE", temporary / "LICENSE")
        shutil.copy2(repository / "docs/LICENSING.md", temporary / "LICENSING.md")
        shutil.copy2(
            repository / "omnivox-tgspeechbox-sys/source-inputs.json",
            temporary / "source-inputs.json",
        )
        notices = temporary / "third-party-licenses"
        notices.mkdir()
        shutil.copy2(source / "LICENSE", notices / "TGSpeechBox-LICENSE.txt")
        shutil.copy2(
            espeak_output / "espeak-ng/src/ucd-tools/COPYING",
            notices / "eSpeak-NG-GPL-3.0.txt",
        )
        shutil.copy2(
            espeak_output / "espeak-ng/src/ucd-tools/COPYING.UCD",
            notices / "Unicode-Data-License.txt",
        )
        shutil.copy2(repository / "Cargo.lock", notices / "omnivox-Cargo.lock")
        (temporary / "README.md").write_text(companion_readme(target), encoding="utf-8")
        inventories = {
            sample_rate: generate_voice_inventory(
                temporary / executable.name,
                marker["lock_file_sha256"],
                sample_rate,
            )
            for sample_rate in SUPPORTED_SAMPLE_RATES
        }
        for sample_rate, inventory in inventories.items():
            (temporary / VOICE_INVENTORY_FILENAMES[sample_rate]).write_text(
                json.dumps(inventory, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
        default_inventory = inventories[DEFAULT_SAMPLE_RATE]
        (temporary / VOICE_INVENTORY_FILENAME).write_text(
            json.dumps(default_inventory, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        provenance = {
            "schema_version": 1,
            "artifact": f"omnivox-tgspeechbox-companion-{suffix}",
            "target": target,
            "markers_advertised": False,
            "rate_mapping": "provisional",
            "default_native_sample_rate_hz": DEFAULT_SAMPLE_RATE,
            "native_sample_rates_hz": list(SUPPORTED_SAMPLE_RATES),
            "tgspeechbox": {
                "release": RELEASE,
                "commit": COMMIT,
                "archive_sha256": marker["archive_sha256"],
                "source_tree_sha256": marker["source_tree_sha256"],
                "packs_included": True,
                "verified_before_build": True,
            },
            "source_input_lock_sha256": marker["lock_file_sha256"],
            "voice_inventory": {
                "file": VOICE_INVENTORY_FILENAME,
                "rate_specific_files": [
                    VOICE_INVENTORY_FILENAMES[sample_rate]
                    for sample_rate in SUPPORTED_SAMPLE_RATES
                ],
                "schema_version": VOICE_INVENTORY_SCHEMA_VERSION,
                "voices": EXPECTED_VOICE_COUNT,
                "generated_by_packaged_helper": True,
            },
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
                    f"TGSpeechBox companion destination is not a directory: {destination}"
                )
            shutil.rmtree(destination)
        temporary.replace(destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    print(f"Staged TGSpeechBox companion in {destination}", file=sys.stderr)
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
        executable, espeak_output = build(arguments, source)
        stage(repository, executable, espeak_output, target, source, marker)
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
