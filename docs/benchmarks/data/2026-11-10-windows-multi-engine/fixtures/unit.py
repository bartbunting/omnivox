#!/usr/bin/env python3
"""Retain exact native test output and exit status; use only private payloads."""
import argparse, hashlib, json, os, subprocess
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument('tests',type=Path);p.add_argument('runtime',type=Path);p.add_argument('output',type=Path)
p.add_argument('filters',nargs='+');p.add_argument('--ignored',action='store_true')
a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
def windows(path):return subprocess.check_output(['wslpath','-w',str(path.resolve())],text=True).strip()
env=dict(os.environ)
env.update(OMNIVOX_TEST_GUARD_EXECUTABLE=windows(a.runtime),
           OMNIVOX_TEST_GUARD_HELPER=windows(a.runtime.parent/'flite/omnivox-flite-helper.exe'))
env['WSLENV']='OMNIVOX_TEST_GUARD_EXECUTABLE:OMNIVOX_TEST_GUARD_HELPER'
rows=[]
for n,pattern in enumerate(a.filters):
 command=[str(a.tests.resolve()),pattern,'--nocapture','--test-threads=1']+(['--include-ignored'] if a.ignored else [])
 result=subprocess.run(command,env=env,capture_output=True,timeout=180)
 (a.output/f'{n}.stdout').write_bytes(result.stdout);(a.output/f'{n}.stderr').write_bytes(result.stderr)
 rows.append(dict(command=command,exit=result.returncode))
 print(pattern,result.returncode,result.stdout.decode(errors='replace')[-1500:],result.stderr.decode(errors='replace')[-1500:],flush=True)
(a.output/'result.json').write_text(json.dumps(dict(tests_sha256=hashlib.sha256(a.tests.read_bytes()).hexdigest(),runtime_sha256=hashlib.sha256(a.runtime.read_bytes()).hexdigest(),runs=rows),indent=2))
raise SystemExit(any(r['exit'] for r in rows))
