#!/usr/bin/env python3
"""Fresh paired Emacs public-command integration; no live profile is contacted."""
import argparse,os,subprocess,sys
from pathlib import Path
sys.dont_write_bytecode=True
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'2026-11-10-resident-reload'))
import resident
parser=argparse.ArgumentParser();parser.add_argument('exe',type=Path);parser.add_argument('output',type=Path);parser.add_argument('--bytecode',action='store_true');parser.add_argument('--engine',default='flite');parser.add_argument('--baseline',action='store_true');parser.add_argument('--source-root',type=Path,default=Path('/home/bart/src/emacsvox-lightweight-reload'));parser.add_argument('--skip-crash',action='store_true');parser.add_argument('--lose-retirement-reply',action='store_true');parser.add_argument('--damage-receipt',action='store_true');args=parser.parse_args()
root=args.output.resolve();root.mkdir(parents=True)
resident.save(root/'config.json',resident.configuration(engine=args.engine))
env={key:os.environ[key] for key in ('PATH','HOME','LANG','WSL_INTEROP','XDG_RUNTIME_DIR') if key in os.environ}
env.update(OMNIVOX_TEST_SOURCE_ROOT=str(args.source_root.resolve()),OMNIVOX_TEST_FIXTURE_DIR=str(Path(__file__).resolve().parent),OMNIVOX_TEST_CONFIG_DIR=str(root),OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE=args.engine,OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_PROGRAM=str(args.exe.resolve()),OMNIVOX_LOG_DIRECTORY=str(root/'logs'))
if args.engine=='piper':
    env.update(OMNIVOX_PIPER_HELPER=str(Path('target/lightweight-investigation/piper-companion/omnivox-piper-helper').resolve()),OMNIVOX_PIPER_MODEL='/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx')
if args.bytecode:env['OMNIVOX_TEST_BYTECODE']='1'
if args.baseline:env['OMNIVOX_TEST_EXPECT_BUG']='1'
if args.skip_crash:env['OMNIVOX_TEST_SKIP_CRASH']='1'
if args.lose_retirement_reply or args.damage_receipt:env['OMNIVOX_TEST_LOSE_RETIREMENT_REPLY']='1'
if args.damage_receipt:env['OMNIVOX_TEST_DAMAGE_RETIREMENT_RECEIPT']='1'
if args.exe.suffix.lower()=='.exe':
    # The launcher's voice-root contract is already-native, unlike the WSL
    # configuration editor path used by this fixture to write its saved file.
    env['OMNIVOX_CONFIG_DIR']=subprocess.check_output(['wslpath','-w',str(root)],text=True).strip()
    env['OMNIVOX_VOICE_ROOT']=subprocess.check_output(['wslpath','-w',str(root/'voices')],text=True).strip()
p=subprocess.run(['/home/bart/opt/emacs-31/bin/emacs','-Q','--batch','-l','/home/bart/src/emacsvox-lightweight-reload/test/run-retained-recovery.el'],env=env,capture_output=True,timeout=90)
(root/'stdout.log').write_bytes(p.stdout);(root/'stderr.log').write_bytes(p.stderr)
assert p.returncode==0,p.stderr.decode()[-2000:]
print(p.stdout.decode())
