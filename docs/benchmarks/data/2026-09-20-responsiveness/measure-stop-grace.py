import json,os,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parent
script=Path('/home/bart/src/omnivox/docs/benchmarks/data/2026-09-20-letter-playback-reserve/measure-bursts.py')
os.environ['OMNIVOX_DECTALK_HELPER']=subprocess.check_output(['wslpath','-w',str(root/'stop-probe/bin/OmnivoxDectalkHelper32.exe')],text=True).strip()
os.environ['WSLENV']=os.environ.get('WSLENV','')+':OMNIVOX_PROBE_STOP_GRACE'
manifest=[]
for repeat in (0,1):
 for grace in ([0,5] if repeat==0 else [5,0]):
  name=f'grace-{grace}-{repeat}'
  os.environ['OMNIVOX_PROBE_STOP_GRACE']=str(grace)
  cmd=[sys.executable,str(script),str(root/name),'--program','/mnt/c/Users/bart/AppData/Local/Emacsvox/Omnivox/runtime/577414114415d7b0/omnivox.exe']
  print('starting',name,flush=True)
  with (root/(name+'.log')).open('w') as log:r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
  manifest.append({'name':name,'command':cmd,'exit_code':r.returncode,'grace_ms':grace})
  (root/'stop-probe-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
  if r.returncode:raise RuntimeError(name)
  d=json.loads((root/name/'results.json').read_text())
  print(name,'last',[round(d['source_ms_by_epoch'][str(i)],1) for i in [60,120,180,240,300]],'starts',[x['started'] for x in d['summary']],flush=True)
