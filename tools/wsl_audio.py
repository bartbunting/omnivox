#!/usr/bin/env python3
"""Prepare and inspect opt-in Windows/Linux Emacsvox audio trials under WSLg."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import sysconfig
import time
from typing import Any


ALSA_CONFIG = """pcm.!default {
    type pulse
    hint.description "Omnivox WSLg audio trial"
}
ctl.!default {
    type pulse
}
"""
RUNTIME_PATH_VARIABLES = (
    "ESPEAK_NG_DATA",
    "OMNIVOX_ELOQUENCE_HELPER", "OMNIVOX_ECI_DLL",
    "OMNIVOX_DECTALK_HELPER", "OMNIVOX_DECTALK_DLL",
    "OMNIVOX_ECI_LIBRARY", "OMNIVOX_DECTALK_LIBRARY", "OMNIVOX_DECTALK_DICTIONARY",
    "OMNIVOX_RHVOICE_LIBRARY", "OMNIVOX_RHVOICE_DATA",
    "OMNIVOX_RHVOICE_CONFIG", "OMNIVOX_RHVOICE_RESOURCES",
    "OMNIVOX_RHVOICE_HELPER", "OMNIVOX_FLITE_HELPER",
    "OMNIVOX_FLITE_VOICES", "OMNIVOX_RUTTS_HELPER",
    "OMNIVOX_TGSPEECHBOX_HELPER", "OMNIVOX_TGSPEECHBOX_DATA",
    "OMNIVOX_PIPER_MODEL", "OMNIVOX_PIPER_HELPER",
    "OMNIVOX_PIPER_ESPEAK_DATA",
)
WINDOWS_PATH = re.compile(r"(^|[;:])([a-z]:[\\/]|\\\\)|\.(exe|dll)($|[;:])", re.I)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def latency(value: str) -> str:
    if value == "default":
        return value
    if not re.fullmatch(r"[0-9]{1,4}", value) or not 50 <= int(value) <= 1000:
        raise ValueError("latency must be 'default' or 50–1000 ms; smaller ALSA requests stalled")
    return str(int(value))


def executable(path: Path) -> Path:
    path = path.expanduser().resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"Executable is not runnable: {path}")
    return path


def plugin_directory(directory: Path | None) -> Path:
    if directory is not None:
        candidates = [directory.expanduser().resolve()]
    else:
        # WSL speech setups can have both 32-bit and 64-bit ALSA plugins.
        # Do not pick an unrelated architecture merely because it sorts first.
        multiarch = sysconfig.get_config_var("MULTIARCH")
        candidates = [
            Path(root) / multiarch / "alsa-lib"
            for root in ("/usr/lib", "/lib") if multiarch
        ]
        candidates.extend(Path(root) / "alsa-lib"
                          for root in ("/usr/lib", "/lib"))
        if sys.maxsize > 2**32:
            candidates.extend(Path(root) / "alsa-lib"
                              for root in ("/usr/lib64", "/lib64"))
    for candidate in candidates:
        if all((candidate / f"libasound_module_{kind}_pulse.so").is_file()
               for kind in ("pcm", "ctl")):
            return candidate.resolve()
    raise ValueError(
        "ALSA PulseAudio plugins are missing. Supply --alsa-plugin-dir from a "
        "compatible installed or privately extracted package; see docs/WSL-AUDIO.md."
    )


def validate_pulse_server(server: str) -> None:
    if not server.startswith("unix:/"):
        raise ValueError("--pulse-server must name an absolute unix: socket")
    path = Path(server[5:])
    if not path.exists() or not stat.S_ISSOCK(path.stat().st_mode):
        raise ValueError(f"WSLg PulseAudio socket is unavailable: {path}")


def pulse_health(env: dict[str, str], timeout: float = 3) -> dict[str, Any]:
    """Check the control connection without playing audio or listing other clients."""
    server = env.get("PULSE_SERVER", "unix:/mnt/wslg/PulseServer")
    result: dict[str, Any] = {
        "server": server, "status": "unverified", "playback_verified": False,
    }
    pactl = shutil.which("pactl", path=env.get("PATH"))
    if pactl is None:
        result["detail"] = "pactl is unavailable; socket presence alone does not establish server health."
        return result
    started = time.monotonic()
    try:
        probe = subprocess.run(
            [pactl, f"--server={server}", "info"], env=env,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=timeout, check=False,
        )
        result["status"] = "reachable" if probe.returncode == 0 else "unavailable"
        result["detail"] = (
            "PulseAudio answered a control query; audible playback has not been tested."
            if probe.returncode == 0 else
            "PulseAudio rejected or could not complete the connection; check the server and access settings."
        )
        result["exit_code"] = probe.returncode
    except subprocess.TimeoutExpired:
        result["status"] = "timeout"
        result["detail"] = (
            f"PulseAudio did not answer within {timeout:g} seconds. "
            "Check the shared WSLg audio server/bridge before restarting speech; "
            "both Linux backends depend on it."
        )
    except OSError as error:
        result["detail"] = f"Could not run pactl: {error}"
    result["elapsed_ms"] = round((time.monotonic() - started) * 1000, 2)
    return result


def environment(
    config: dict[str, Any], directory: Path, target: str,
    inherited: dict[str, str] | None = None, request: str | None = None,
) -> tuple[dict[str, str], list[str]]:
    env = dict(os.environ if inherited is None else inherited)
    env.pop("EMACSVOX_OMNIVOX_PROBE_ONLY", None)
    env.update({
        "EMACSVOX_DIR": config["emacsvox_dir"],
        "TTS_PROGRAM": "omnivox",
        "OMNIVOX_AUDIO_OUTPUT": "device",
        "OMNIVOX_ENGINE": "espeak",
        "OMNIVOX_LOG_DIRECTORY": str(directory / f"{target}-logs"),
    })
    removed = []
    if target == "linux":
        backend = env.get("OMNIVOX_WSL_AUDIO_OUTPUT", "device").strip().lower()
        if backend not in ("device", "pulse"):
            raise ValueError("OMNIVOX_WSL_AUDIO_OUTPUT must be device or pulse")
        env["OMNIVOX_AUDIO_OUTPUT"] = backend
        for name in RUNTIME_PATH_VARIABLES:
            if WINDOWS_PATH.search(env.get(name, "")):
                removed.append(name)
                env.pop(name)
        env.update({
            "OMNIVOX_PROGRAM": config["linux_program"],
            "ALSA_CONFIG_PATH": str(directory / "alsa.conf"),
            "ALSA_PLUGIN_DIR": config["alsa_plugin_dir"],
            "PULSE_SERVER": config["pulse_server"],
        })
        selected = ("default" if backend == "pulse" else
                    latency(request if request is not None else
                            env.get("OMNIVOX_WSL_LATENCY_MS", config["latency_ms"])))
        if backend == "pulse":
            env.pop("ALSA_CONFIG_PATH", None)
            env.pop("ALSA_PLUGIN_DIR", None)
            # The general native backend starts at 20 ms. Repeated WSLg
            # short-tone capture needed more headroom; keep the trial tunable.
            env.setdefault("OMNIVOX_PULSE_LATENCY_MS", "40")
        if selected == "default":
            env.pop("PULSE_LATENCY_MSEC", None)
        else:
            env["PULSE_LATENCY_MSEC"] = selected
        player = shutil.which("paplay", path=env.get("PATH"))
        if player:
            env["EMACSVOX_PLAY"] = player
        else:
            env.pop("EMACSVOX_PLAY", None)
    else:
        # Leave automatic selection empty so the existing launcher still loads
        # its matching staged runtime metadata and handles WSLENV itself.
        env["OMNIVOX_PROGRAM"] = config["windows_program"] or ""
    return env, removed


def resolve_program(config: dict[str, Any], env: dict[str, str], target: str) -> Path:
    launcher = Path(config["emacsvox_dir"]) / "servers/omnivox"
    result = subprocess.run(
        [str(launcher)], env={**env, "EMACSVOX_OMNIVOX_PROBE_ONLY": "1"},
        stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10,
        check=True,
    )
    program = executable(Path(result.stdout.strip()))
    if (program.suffix.lower() == ".exe") != (target == "windows"):
        raise ValueError(f"{target} trial resolved the wrong platform: {program}")
    return program


def identity(config: dict[str, Any], directory: Path, target: str) -> dict[str, Any]:
    env, removed = environment(config, directory, target)
    program = resolve_program(config, env, target)
    result = subprocess.run(
        [str(Path(config["emacsvox_dir"]) / "servers/omnivox"), "--version"],
        env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
        timeout=10, check=True,
    )
    return {
        "program": str(program),
        "sha256": sha256(program),
        "version": result.stdout.strip(),
        "configured_audio_path": (
            ("native libpulse -> WSLg PulseAudio -> RDP -> Windows device"
             if env["OMNIVOX_AUDIO_OUTPUT"] == "pulse" else
             "Rodio/CPAL -> ALSA PulseAudio plugin -> WSLg PulseAudio -> RDP -> Windows device")
            if target == "linux" else "Rodio/CPAL -> Windows shared-mode WASAPI"
        ),
        "audio_output": env["OMNIVOX_AUDIO_OUTPUT"],
        "native_pulse_latency_ms": env.get("OMNIVOX_PULSE_LATENCY_MS", "20")
        if env["OMNIVOX_AUDIO_OUTPUT"] == "pulse" else None,
        "audio_path_verified_by_capture": False,
        "engine_preference": env["OMNIVOX_ENGINE"],
        "removed_windows_path_variables": removed,
        "pulse_server": env.get("PULSE_SERVER") if target == "linux" else None,
        "pulse_latency_ms": env.get("PULSE_LATENCY_MSEC", "default")
        if target == "linux" else None,
        "alsa_config": env.get("ALSA_CONFIG_PATH") if target == "linux" else None,
        "alsa_plugin_dir": env.get("ALSA_PLUGIN_DIR") if target == "linux" else None,
    }


def read_config(directory: Path) -> dict[str, Any]:
    config = json.loads((directory / "profile.json").read_text(encoding="utf-8"))
    if config.get("profile_version") != 1:
        raise ValueError("Unsupported WSL audio profile version; prepare a new directory")
    return config


def report(config: dict[str, Any], directory: Path) -> dict[str, Any]:
    identities = {target: identity(config, directory, target)
                  for target in ("windows", "linux")}
    uses_alsa = identities["linux"]["audio_output"] == "device"
    return {
        "report_version": 1,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "host": platform.platform(),
        "tool_sha256": sha256(Path(__file__)),
        "profile_sha256": sha256(directory / "profile.json"),
        "alsa_config_sha256": sha256(directory / "alsa.conf") if uses_alsa else None,
        "alsa_plugin_sha256": {
            kind: sha256(Path(config["alsa_plugin_dir"]) / f"libasound_module_{kind}_pulse.so")
            for kind in ("pcm", "ctl")
        } if uses_alsa else None,
        "runtimes": identities,
        "linux_server": pulse_health(environment(config, directory, "linux")[0]),
        "versions_match": identities["linux"]["version"] == identities["windows"]["version"],
        "acoustic_onset_measured": False,
        "stop_to_silence_measured": False,
        "comparison_requirement": (
            "Match build provenance, exact voice, rate, text and device before "
            "drawing cross-platform latency conclusions; version equality alone is insufficient."
        ),
    }


def write_json(path: Path, value: Any) -> None:
    with path.open("x", encoding="utf-8") as output:
        json.dump(value, output, indent=2, ensure_ascii=False)
        output.write("\n")


def prepare(args: argparse.Namespace) -> None:
    directory = args.directory.expanduser().resolve()
    if directory.exists():
        raise ValueError(f"Refusing existing trial directory: {directory}")
    emacsvox = args.emacsvox_dir.expanduser().resolve()
    for relative in ("bin/emacsvox", "servers/omnivox"):
        executable(emacsvox / relative)
    program = executable(args.linux_program)
    if program.suffix.lower() == ".exe":
        raise ValueError("--linux-program requires a Linux executable")
    for relative in ("espeak-ng-data", "third-party-licenses"):
        if not (program.parent / relative).is_dir():
            raise ValueError(f"Missing {relative} beside {program}; stage with make build")
    validate_pulse_server(args.pulse_server)
    config = {
        "profile_version": 1,
        "emacsvox_dir": str(emacsvox),
        "linux_program": str(program),
        "windows_program": str(executable(args.windows_program))
        if args.windows_program else None,
        "alsa_plugin_dir": str(plugin_directory(args.alsa_plugin_dir)),
        "pulse_server": args.pulse_server,
        "latency_ms": latency(args.latency_ms),
    }
    # Check selection before writing the trial; no audio or package installation.
    for target in ("linux", "windows"):
        env, _ = environment(config, directory, target)
        resolve_program(config, env, target)
    directory.mkdir(mode=0o700, parents=True)
    write_json(directory / "profile.json", config)
    (directory / "alsa.conf").write_text(ALSA_CONFIG, encoding="utf-8")
    for target in ("linux", "windows"):
        command = [sys.executable, str(Path(__file__).resolve()), "run",
                   str(directory), target]
        launcher = directory / f"emacsvox-{target}-test"
        launcher.write_text(
            "#!/bin/sh\n# Generated opt-in WSLg trial; existing settings are unchanged.\n"
            f'exec {shlex.join(command)} "$@"\n', encoding="utf-8",
        )
        launcher.chmod(0o700)
    print(f"Prepared {directory}")
    print("Launch emacsvox-windows-test or emacsvox-linux-test from that directory.")
    print("Use --report to inspect both runtimes, --backend for protocol tools, or -nw for Emacs.")


def launch(directory: Path, target: str, arguments: list[str]) -> None:
    config = read_config(directory)
    if arguments[:1] == ["--report"]:
        if len(arguments) != 1:
            raise ValueError("--report takes no additional arguments")
        print(json.dumps(report(config, directory), indent=2))
        return
    env, _ = environment(config, directory, target)
    resolve_program(config, env, target)
    if target == "linux":
        validate_pulse_server(config["pulse_server"])
        if env["OMNIVOX_AUDIO_OUTPUT"] == "device":
            plugin_directory(Path(config["alsa_plugin_dir"]))
    backend = arguments[:1] == ["--backend"]
    if backend:
        arguments = arguments[1:]
    launcher = Path(config["emacsvox_dir"]) / ("servers/omnivox" if backend else "bin/emacsvox")
    os.execve(launcher, [str(launcher), *arguments], env)


def sink_inputs(env: dict[str, str], pid: int) -> list[dict[str, Any]]:
    result = subprocess.run(
        ["pactl", "--format=json", "list", "sink-inputs"],
        env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True,
        timeout=5, check=True,
    )
    # Retain only the owned stream and measurement fields, not other clients'
    # speech, process IDs, usernames, hostnames or machine identifiers.
    fields = ("sample_specification", "buffer_latency_usec", "sink_latency_usec",
              "resample_method", "corked")
    entries = [
        {field: entry.get(field) for field in fields}
        for entry in json.loads(result.stdout)
        if str(entry.get("properties", {}).get("application.process.id")) == str(pid)
    ]
    for entry in entries:
        for field in ("buffer_latency_usec", "sink_latency_usec"):
            value = entry[field]
            if isinstance(value, bool) or not isinstance(value, (int, float)) \
                    or not math.isfinite(value) or value < 0:
                raise ValueError(f"pactl did not supply a valid {field} for the owned stream")
    return entries


def stop_process(process: subprocess.Popen) -> bool:
    """Close input; bound shutdown and retire only the trial's POSIX process group."""
    if process.stdin and not process.stdin.closed:
        process.stdin.close()
    try:
        process.wait(timeout=3)
        return True
    except subprocess.TimeoutExpired:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
            if sig == signal.SIGTERM:
                # Give helpers a chance to exit too, even if their owner exits first.
                time.sleep(0.1)
        process.wait(timeout=3)
        return False


def probe(config: dict[str, Any], directory: Path, output: Path, samples: int) -> bool:
    if os.environ.get("OMNIVOX_WSL_AUDIO_OUTPUT", "device").strip().lower() != "device":
        raise ValueError("The idle buffer probe measures the ALSA device path; native PulseAudio corks idle streams")
    if not 1 <= samples <= 120:
        raise ValueError("--samples must be from 1 through 120")
    validate_pulse_server(config["pulse_server"])
    plugin_directory(Path(config["alsa_plugin_dir"]))
    if not shutil.which("pactl"):
        raise ValueError("The buffer probe requires pactl (PulseAudio utilities)")
    metadata = report(config, directory)
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    write_json(output / "runtimes.json", metadata)
    results: dict[str, Any] = {
        "report_version": 1,
        "measurement": "idle PulseAudio stream introspection, not acoustic latency",
        "acoustic_onset_measured": False,
        "stop_to_silence_measured": False,
        "sample_interval_seconds": 0.5,
        "settle_after_stream_discovery_seconds": 0.5,
        "shutdown_deadline_seconds": 3,
        "completed": False,
        "runs": [],
    }
    ok = True
    try:
        for request in ("default", config["latency_ms"]):
            if any(run["pulse_latency_ms"] == request for run in results["runs"]):
                continue
            env, _ = environment(config, directory, "linux", request=request)
            command = [str(Path(config["emacsvox_dir"]) / "servers/omnivox")]
            run: dict[str, Any] = {"pulse_latency_ms": request, "samples": []}
            results["runs"].append(run)
            with (output / f"probe-{request}.stderr").open("x") as errors:
                process = subprocess.Popen(
                    command, env=env, stdin=subprocess.PIPE,
                    stdout=subprocess.DEVNULL, stderr=errors, start_new_session=True,
                )
                try:
                    # Bounded discovery: the server may still be initializing helpers.
                    deadline = time.monotonic() + 15
                    while True:
                        entries = sink_inputs(env, process.pid)
                        if entries:
                            break
                        if process.poll() is not None or time.monotonic() >= deadline:
                            raise RuntimeError("No PulseAudio stream appeared for the owned process")
                        time.sleep(0.1)
                    time.sleep(0.5)
                    entries = sink_inputs(env, process.pid)
                    for index in range(samples):
                        if index:
                            time.sleep(0.5)
                            entries = sink_inputs(env, process.pid)
                        run["samples"].append({
                            "observed_at": datetime.now(timezone.utc).isoformat(),
                            "observed_monotonic_ns": time.monotonic_ns(),
                            "sink_inputs": entries,
                        })
                        if not entries:
                            raise RuntimeError("The owned PulseAudio stream disappeared")
                except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
                    run["error"] = str(error)
                finally:
                    run["shutdown_within_deadline"] = stop_process(process)
                    run["exit_code"] = process.returncode
            run["passed"] = (
                "error" not in run and run["shutdown_within_deadline"]
                and run["exit_code"] == 0
            )
            ok = ok and run["passed"]
        results["completed"] = True
    finally:
        write_json(output / "buffers.json", results)
    print(f"Buffer probe {'passed' if ok else 'failed'}; evidence: {output}")
    return ok


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    setup = commands.add_parser("prepare", help="create an isolated trial directory")
    setup.add_argument("directory", type=Path)
    setup.add_argument("--emacsvox-dir", type=Path, required=True)
    setup.add_argument("--linux-program", type=Path, required=True)
    setup.add_argument("--windows-program", type=Path)
    setup.add_argument("--alsa-plugin-dir", type=Path)
    setup.add_argument("--pulse-server", default="unix:/mnt/wslg/PulseServer")
    setup.add_argument("--latency-ms", default="50")
    run = commands.add_parser("run", help="internal entry point used by generated launchers")
    run.add_argument("directory", type=Path)
    run.add_argument("target", choices=("linux", "windows"))
    run.add_argument("arguments", nargs=argparse.REMAINDER)
    inspect = commands.add_parser("report", help="report both runtime identities without audio")
    inspect.add_argument("directory", type=Path)
    health = commands.add_parser("health", help="check the Linux audio server with a three-second deadline")
    health.add_argument("--pulse-server", default=os.environ.get("PULSE_SERVER", "unix:/mnt/wslg/PulseServer"))
    buffers = commands.add_parser("probe", help="sample Linux default and trial buffers")
    buffers.add_argument("directory", type=Path)
    buffers.add_argument("output", type=Path)
    buffers.add_argument("--samples", type=int, default=4)
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args)
        elif args.command == "run":
            launch(args.directory.resolve(), args.target, args.arguments)
        elif args.command == "report":
            print(json.dumps(report(read_config(args.directory), args.directory.resolve()), indent=2))
        elif args.command == "health":
            result = pulse_health({**os.environ, "PULSE_SERVER": args.pulse_server})
            print(json.dumps(result, indent=2))
            return 0 if result["status"] == "reachable" else 1
        elif not probe(read_config(args.directory), args.directory.resolve(),
                       args.output.expanduser().resolve(), args.samples):
            return 1
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"WSL audio trial: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("WSL audio trial interrupted; probe cleanup completed.", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
