#!/usr/bin/env python3
import os,subprocess,sys
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[1]).resolve();root.mkdir(parents=True)
env=dict(os.environ,OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex(),OMNIVOX_PROGRAM=str(Path('target/lightweight-investigation/payload-candidate/omnivox').resolve()),OMNIVOX_LOG_DIRECTORY=str(root/'logs'))
probe.save(root/'config.json',probe.configuration())
command=['/home/bart/opt/emacs-31/bin/emacs','-Q','--batch','-l','/home/bart/src/emacsvox-lightweight-reload/test/run-lightweight-reload.el']
p=subprocess.run(command,env=env,capture_output=True,timeout=90)
(root/'stdout.log').write_bytes(p.stdout);(root/'stderr.log').write_bytes(p.stderr)
assert p.returncode==0,p.stderr.decode()
print(p.stdout.decode())
