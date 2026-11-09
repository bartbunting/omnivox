#!/usr/bin/env python3
"""Fresh paired Emacs public-command integration; no live profile is contacted."""
import argparse,os,subprocess
from pathlib import Path
import resident
parser=argparse.ArgumentParser();parser.add_argument('exe',type=Path);parser.add_argument('output',type=Path);parser.add_argument('--bytecode',action='store_true');parser.add_argument('--engine',default='espeak');args=parser.parse_args()
root=args.output.resolve();root.mkdir(parents=True)
resident.save(root/'config.json',resident.configuration(engine=args.engine))
env={key:os.environ[key] for key in ('PATH','HOME','LANG','WSL_INTEROP','XDG_RUNTIME_DIR') if key in os.environ}
env.update(OMNIVOX_TEST_CONFIG_DIR=str(root),OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE=args.engine,OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_PROGRAM=str(args.exe.resolve()),OMNIVOX_LOG_DIRECTORY=str(root/'logs'))
if args.bytecode:env['OMNIVOX_TEST_BYTECODE']='1'
if args.exe.suffix.lower()=='.exe':
    # The launcher's voice-root contract is already-native, unlike the WSL
    # configuration editor path used by this fixture to write its saved file.
    env['OMNIVOX_CONFIG_DIR']=subprocess.check_output(['wslpath','-w',str(root)],text=True).strip()
    env['OMNIVOX_VOICE_ROOT']=subprocess.check_output(['wslpath','-w',str(root/'voices')],text=True).strip()
p=subprocess.run(['/home/bart/opt/emacs-31/bin/emacs','-Q','--batch','-l','/home/bart/src/emacsvox-lightweight-reload/test/run-resident-reload.el'],env=env,capture_output=True,timeout=90)
(root/'stdout.log').write_bytes(p.stdout);(root/'stderr.log').write_bytes(p.stderr)
assert p.returncode==0,p.stderr.decode()[-2000:]
print(p.stdout.decode())
