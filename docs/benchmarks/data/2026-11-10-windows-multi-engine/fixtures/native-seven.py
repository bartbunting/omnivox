#!/usr/bin/env python3
import json,subprocess,sys
from pathlib import Path
metadata=json.load(open(sys.argv[1]));root=Path(sys.argv[2])
def win(path):return subprocess.check_output(['wslpath','-w',str(path)],text=True).strip()
command=['/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',win(Path(__file__).with_suffix('.ps1').resolve()),'-Executable',win(metadata['executable']),'-Data',metadata['data'],'-Root',win(root)]
result=subprocess.run(command,timeout=1200)
raise SystemExit(result.returncode)
