"""Reproduce the chunk-size comparison with a staged native payload.

Run from the Omnivox root: python3 PATH/TO/reproduce.py SERVER OUTPUT_DIRECTORY.
Use native paths on Windows, or run the native Python interpreter there.
Unset ESPEAK_NG_DATA for adjacent-data payloads; if data is staged separately,
set it to that payload's matching data parent (recorded in its .path file).
The retained Linux runs used a debug payload while a Windows build ran; new
timings should use an idle machine. Outputs are new observations, not replacements
for the original reports. No device is opened.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

server, output = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
output.mkdir(parents=True, exist_ok=False)
shipped = ["winrt", "macos", "piper", "rhvoice", "flite", "rutts",
           "tgspeechbox", "eloquence", "dectalk", "mbrola"]
environment = {key: value for key, value in os.environ.items()
               if not key.startswith("OMNIVOX_")}
voice = r"espeak:gmw\en-US" if os.name == "nt" else "espeak:gmw/en-US"
for limit in (5, 15, 30):
    root = output / str(limit)
    root.mkdir()
    (root / "config.json").write_text(json.dumps(dict(
        schema=2, speech=dict(max_chunk_words=limit),
        engine_overrides={name: dict(enabled=False) for name in shipped})))
for index, limit in enumerate((15, 5, 30, 30, 5, 15)):
    subprocess.run([
        sys.executable, "tools/benchmark_server.py", str(server),
        "--null-audio", "--engine", "espeak", "--expected-engine-id", "espeak",
        "--voice-id", voice, "--mode", "warm", "--case", "line",
        "--case", "replacement", "--iterations", "10", "--warmups", "2",
        "--server-arg=--config-dir", "--server-arg=" + str(output / str(limit)),
        "--json-output", str(output / f"limit-{limit}-run-{index}.json"),
    ], env=environment, check=True, timeout=180)
