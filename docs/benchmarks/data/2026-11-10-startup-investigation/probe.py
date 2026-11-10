import sys,os,json,time,hashlib,subprocess,statistics
from pathlib import Path
sys.dont_write_bytecode=True
repo=Path('/home/bart/src/omnivox-lightweight-reload')
sys.path.insert(0,str(repo/'docs/benchmarks/data/2026-11-10-replacement-and-handover/fixtures'))
from common import Peer
meta=json.loads((repo/'docs/benchmarks/data/2026-11-10-helper-queue/listening/runtime.json').read_text())
root=Path('/mnt/c/Users/bart/AppData/Local/Temp')/('omnivox-startup-investigation-'+str(time.time_ns()));root.mkdir()
def win(p):return subprocess.check_output(['wslpath','-w',str(p)],text=True).strip()
rows=[]
for block in range(6):
 order=['all','without_tg','selected_pair']
 if block%2:order.reverse()
 for profile in order:
  case=root/f'{block}-{profile}';case.mkdir()
  disabled=['macos','piper','mbrola']
  if profile=='without_tg': disabled+=['tgspeechbox']
  if profile=='selected_pair':disabled+=['winrt','eloquence','dectalk','rhvoice','rutts','tgspeechbox']
  (case/'config.json').write_text(json.dumps(dict(schema=4,engine_overrides={k:dict(enabled=False) for k in disabled})))
  env={k:os.environ[k] for k in ('PATH','HOME','LANG','WSL_INTEROP') if k in os.environ}
  env.update(OMNIVOX_CONFIG_DIR=win(case),OMNIVOX_VOICE_ROOT=win(case/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null',ESPEAK_NG_DATA=meta['data'],OMNIVOX_RHVOICE_LIBRARY=meta['rhvoice_library'],OMNIVOX_RHVOICE_DATA=meta['rhvoice_data'],OMNIVOX_STARTUP_TIMING='1',RUST_LOG='info')
  env['WSLENV']=':'.join(k for k in env if k.startswith('OMNIVOX_') or k in ('ESPEAK_NG_DATA','RUST_LOG'))
  peer=Peer([meta['executable'],'--voice-library-owner','--guarded'],env,case,'main','flite')
  try:
   first=peer.speech();steady=[peer.speech() for _ in range(5)]
   row=dict(block=block,profile=profile,ready_ms=(peer.ready-peer.started)/1e6,first_source_ms=(first['start_ns']-peer.started)/1e6+first['source_ms'],speech_ms=[s['source_ms'] for s in steady],directory=str(case))
  finally:exitcode=peer.close()
  assert exitcode==0
  row['cleanup_exit']=exitcode;rows.append(row)
  (root/'result.json').write_text(json.dumps(dict(runtime=meta,rows=rows),indent=2)+'\n')
  print(json.dumps(row),flush=True)
print('ROOT',root,flush=True)
for profile in ['all','without_tg','selected_pair']:
 r=[x for x in rows if x['profile']==profile];print(profile,statistics.median(x['first_source_ms'] for x in r),flush=True)
