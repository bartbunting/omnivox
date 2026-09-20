import json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parent
programs=json.loads((root/'acceptance-manifest.json').read_text())['programs']
script=Path('/home/bart/src/omnivox/docs/benchmarks/data/2026-09-20-letter-playback-reserve/measure-bursts.py')
manifest=[]
for repeat in (1,2):
 for target in (['after','before'] if repeat==1 else ['before','after']):
  name=f'{target}-bursts-{repeat}'
  cmd=[sys.executable,str(script),str(root/name),'--program',programs[target]]
  print('starting',name,flush=True)
  with (root/(name+'.log')).open('w') as log:r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
  manifest.append({'name':name,'command':cmd,'exit_code':r.returncode})
  (root/'repeat-bursts-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
  if r.returncode:raise RuntimeError(name)
  print('passed',name,flush=True)
