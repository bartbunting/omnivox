#!/usr/bin/env python3
"""One fresh compiled recovery comparison, with old and new client paths explicit."""
import json
import argparse
import os
from pathlib import Path
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, str(next(p for p in Path(__file__).resolve().parents if (p / "Cargo.toml").is_file()) / "docs/benchmarks/data/2026-11-10-resident-reload"))
import resident

parser = argparse.ArgumentParser()
parser.add_argument("exe", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--legacy", action="store_true")
parser.add_argument("--profile", action="store_true")
parser.add_argument("--availability", action="store_true")
parser.add_argument("--healthy", action="store_true")
parser.add_argument("--expect-unsupported", action="store_true")
parser.add_argument("--engine", default="flite")
args = parser.parse_args()
root = args.output.resolve()
root.mkdir(parents=True, exist_ok=False)
resident.save(root / "config.json", dict(schema=4,engine_overrides={k:dict(enabled=False) for k in ["macos","piper","mbrola"]}))
env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "WSL_INTEROP", "XDG_RUNTIME_DIR") if key in os.environ}
env.update(OMNIVOX_PROGRAM=str(args.exe.resolve()), OMNIVOX_ENGINE=args.engine,
           OMNIVOX_TEST_FIXTURE_DIR=str(next(p for p in Path(__file__).resolve().parents if (p / "Cargo.toml").is_file()) / "docs/benchmarks/data/2026-11-10-emergency-speech"),
           OMNIVOX_TEST_SOURCE_ROOT="/home/bart/src/emacsvox-lightweight-reload/",
           OMNIVOX_TEST_CONFIG_DIR=str(root), OMNIVOX_CONFIG_DIR=str(root),
           OMNIVOX_TEST_VOICE_ROOT=str(root / "voices"), OMNIVOX_VOICE_ROOT=str(root / "voices"),
           OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_LOG_DIRECTORY=str(root / "logs"), OMNIVOX_TEST_BYTECODE=os.environ.get("OMNIVOX_TEST_BYTECODE", ""))
if not env["OMNIVOX_TEST_BYTECODE"]:
    env.pop("OMNIVOX_TEST_BYTECODE")
if args.legacy:
    env["OMNIVOX_TEST_LEGACY_ROOT"] = "/home/bart/src/omnivox-lightweight-reload/target/guard-handoff/baseline-client/"
if args.expect_unsupported:
    env["OMNIVOX_TEST_UNSUPPORTED"] = "1"
if args.profile:
    env.update(OMNIVOX_TEST_PROFILE="1", OMNIVOX_STARTUP_TIMING="1")
    env["WSLENV"] = "OMNIVOX_STARTUP_TIMING"
if args.engine == "piper":
    env.update(OMNIVOX_PIPER_HELPER=str(Path("target/lightweight-investigation/piper-companion/omnivox-piper-helper").resolve()),
               OMNIVOX_PIPER_MODEL="/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx")
if args.exe.suffix.lower() == ".exe":
    env['OMNIVOX_TEST_WINDOWS_DATA']=os.environ['OMNIVOX_TEST_WINDOWS_DATA']
    env['ESPEAK_NG_DATA']=os.environ['OMNIVOX_TEST_WINDOWS_DATA']
    env['WSLENV']='OMNIVOX_CONFIG_DIR:OMNIVOX_VOICE_ROOT:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA:OMNIVOX_STARTUP_TIMING'

    for key in ("OMNIVOX_CONFIG_DIR", "OMNIVOX_VOICE_ROOT"):
        env[key] = subprocess.check_output(["wslpath", "-w", env[key]], text=True).strip()
if os.environ.get('OMNIVOX_TEST_NOTIFICATION_DELAY'):
    env['OMNIVOX_TEST_NOTIFICATION_DELAY']=os.environ['OMNIVOX_TEST_NOTIFICATION_DELAY']
if os.environ.get('OMNIVOX_TEST_STARTUP_BASELINE'):
    env['OMNIVOX_TEST_STARTUP_BASELINE']=os.environ['OMNIVOX_TEST_STARTUP_BASELINE']
if os.environ.get('OMNIVOX_TEST_GUI'):
    env['OMNIVOX_ELOQUENCE_HELPER']=r'C:\Users\bart\AppData\Local\Temp\omnivox-multi-20261110-gui-probe\bin\OmnivoxEloquenceHelper32.exe'
    env['OMNIVOX_DECTALK_HELPER']=r'C:\Users\bart\AppData\Local\Temp\omnivox-multi-20261110-gui-probe\bin\OmnivoxDectalkHelper32.exe'
    env['WSLENV']+=':OMNIVOX_ELOQUENCE_HELPER:OMNIVOX_DECTALK_HELPER'
env.update(OMNIVOX_RHVOICE_LIBRARY=os.environ['OMNIVOX_TEST_RHVOICE_LIBRARY'],OMNIVOX_RHVOICE_DATA=os.environ['OMNIVOX_TEST_RHVOICE_DATA']);env['WSLENV']+=':OMNIVOX_RHVOICE_LIBRARY:OMNIVOX_RHVOICE_DATA'
process = subprocess.run(["/home/bart/opt/emacs-31/bin/emacs", "-Q", "--batch", "-l",
                          str(Path(__file__).with_name("restart-availability.el"))], # Match interactive Emacs: batch deliberately leaves inherited SIGPIPE unchanged.
                          env=env, capture_output=True, timeout=240, restore_signals=False)
(root / "exit.json").write_text(json.dumps({"returncode":process.returncode}))
(root / "stdout.log").write_bytes(process.stdout)
(root / "stderr.log").write_bytes(process.stderr)
print(process.stdout.decode())
assert process.returncode == 0, process.stderr.decode()[-4500:]
