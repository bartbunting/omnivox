#!/usr/bin/env python3
"""Fresh isolated Emacs profile using one explicitly qualified private runtime."""
import argparse, hashlib, json, os, subprocess, uuid
from pathlib import Path

here=Path(__file__).resolve().parent
baseline=here.parent.parent/'2026-11-10-replacement-and-handover/listening'
p=argparse.ArgumentParser()
p.add_argument('--engine',choices=('espeak','flite','winrt','eloquence','dectalk','rutts','tgspeechbox','rhvoice'),default='flite')
p.add_argument('--batch-check',action='store_true')
p.add_argument('--gui-check',action='store_true',help='Validate full interactive startup with silent output, then exit')
p.add_argument('--terminal',action='store_true')
a=p.parse_args()
manifest=json.loads((baseline/'runtime.json').read_text())
exe=Path(manifest['executable'])
if hashlib.sha256(exe.read_bytes()).hexdigest()!=manifest['sha256']:
    raise SystemExit('The private speech executable changed; qualification must be checked again.')
source=Path('/home/bart/src/emacsvox-lightweight-reload')
subprocess.run(['make','bytecode-check','EMACS=/home/bart/opt/emacs-31/bin/emacs'],cwd=source,check=True)
root=Path('/mnt/c/Users/bart/AppData/Local/Temp/omnivox-listening-performance-20261110')/str(uuid.uuid4())
root.mkdir(parents=True,exist_ok=False)
for directory in ('emacs','resources','config','state','logs','home','cache'):(root/directory).mkdir()
disabled=('macos','piper','mbrola')
(root/'config.json').write_text(json.dumps(dict(schema=4,engine_overrides={key:dict(enabled=False) for key in disabled if key!=a.engine}),indent=2)+'\n')
def windows(path):return subprocess.check_output(['wslpath','-w',str(path)],text=True).strip()
env=dict(os.environ)
for key in list(env):
    if key.startswith(('OMNIVOX_','SWIFTMAC_','SHARPWIN_')):env.pop(key)
env.update(HOME=str(root/'home'),XDG_CACHE_HOME=str(root/'cache'),EMACSVOX_DIR=str(source),TTS_PROGRAM='omnivox',OMNIVOX_PROGRAM=str(exe),
           OMNIVOX_CONFIG_DIR=windows(root),OMNIVOX_VOICE_ROOT=windows(root/'voices'),
           OMNIVOX_ENGINE=a.engine,OMNIVOX_AUDIO_OUTPUT='null' if a.batch_check or a.gui_check else 'device',
           ESPEAK_NG_DATA=manifest['data'],OMNIVOX_RHVOICE_LIBRARY=manifest['rhvoice_library'],
           OMNIVOX_RHVOICE_DATA=manifest['rhvoice_data'],OMNIVOX_LOG_DIRECTORY=str(root/'logs'),
           XDG_CONFIG_HOME=str(root/'config'),XDG_STATE_HOME=str(root/'state'),
           OMNIVOX_LISTENING_ROOT=str(root),OMNIVOX_LISTENING_ENGINE=a.engine,
           EMACSVOX_PLAY=str(source/'servers/windows-play'))
env['WSLENV']='OMNIVOX_CONFIG_DIR:OMNIVOX_VOICE_ROOT:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA:OMNIVOX_RHVOICE_LIBRARY:OMNIVOX_RHVOICE_DATA'
(root/'runtime.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Private profile:',root,flush=True)
command=['/home/bart/opt/emacs-31/bin/emacs','-Q','--no-splash']
if a.batch_check:command+=['--batch']
elif a.terminal:command+=['-nw']
if a.gui_check:
    # Startup announcements can legitimately replace the tracked check utterance.
    command+=['--eval','(setq emacsvox-speak-messages nil emacsvox-speak-ready-message nil emacsvox-play-startup-icon nil)']
command+=['--load',str(baseline/'startup.el')]
if a.gui_check:command+=['--load',str(here/'check.el')]
os.execve(command[0],command,env)
