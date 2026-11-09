#!/usr/bin/env python3
"""One fresh compiled recovery comparison, with old and new client paths explicit."""
import argparse
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "2026-11-10-resident-reload"))
import resident

parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--legacy", action="store_true")
parser.add_argument("--profile", action="store_true")
parser.add_argument("--expect-unsupported", action="store_true")
parser.add_argument("--engine", default="flite")
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
resident.save(root / "config.json", resident.configuration(engine=args.engine))
env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "WSL_INTEROP", "XDG_RUNTIME_DIR") if key in os.environ}
env.update(OMNIVOX_PROGRAM=str(args.exe.resolve()), OMNIVOX_ENGINE=args.engine,
           OMNIVOX_TEST_FIXTURE_DIR=str(Path(__file__).resolve().parents[1] / "2026-11-10-emergency-speech"),
           OMNIVOX_TEST_SOURCE_ROOT="/home/bart/src/emacsvox-lightweight-reload/",
           OMNIVOX_TEST_CONFIG_DIR=str(root), OMNIVOX_CONFIG_DIR=str(root),
           OMNIVOX_TEST_VOICE_ROOT=str(root / "voices"), OMNIVOX_VOICE_ROOT=str(root / "voices"),
           OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_LOG_DIRECTORY=str(root / "logs"), OMNIVOX_TEST_BYTECODE="1")
if args.legacy:
    env["OMNIVOX_TEST_LEGACY_ROOT"] = "/home/bart/src/omnivox-lightweight-reload/target/deferred-start/baseline-client/"
if args.expect_unsupported:
    env["OMNIVOX_TEST_UNSUPPORTED"] = "1"
if args.profile:
    env.update(OMNIVOX_TEST_PROFILE="1", OMNIVOX_STARTUP_TIMING="1")
    env["WSLENV"] = "OMNIVOX_STARTUP_TIMING"
if args.engine == "piper":
    env.update(OMNIVOX_PIPER_HELPER=str(Path("target/lightweight-investigation/piper-companion/omnivox-piper-helper").resolve()),
               OMNIVOX_PIPER_MODEL="/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx")
if args.exe.suffix.lower() == ".exe":
    for key in ("OMNIVOX_CONFIG_DIR", "OMNIVOX_VOICE_ROOT"):
        env[key] = subprocess.check_output(["wslpath", "-w", env[key]], text=True).strip()
process = subprocess.run(["/home/bart/opt/emacs-31/bin/emacs", "-Q", "--batch", "-l",
                          str(Path(__file__).with_name("run-profile.el"))], env=env, capture_output=True, timeout=120)
(root / "stdout.log").write_bytes(process.stdout)
(root / "stderr.log").write_bytes(process.stderr)
print(process.stdout.decode())
assert process.returncode == 0, process.stderr.decode()[-4500:]
