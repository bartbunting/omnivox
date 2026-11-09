#!/usr/bin/env python3
"""Eight rotated matched blocks; no compilation is launched during this run."""
import json,os,subprocess,sys,time
from pathlib import Path
root=Path(sys.argv[1]).resolve();root.mkdir(parents=True)
probe=Path(__file__).with_name('resident.py').resolve()
release=Path('/home/bart/tmp/ov118-bench/release/omnivox')
maintenance=Path('target/lightweight-investigation/payload-maintenance/omnivox').resolve()
candidate=Path('target/resident-reload/payload-final/omnivox').resolve()
cases=[('release',release,False),('maintenance',maintenance,False),('resident-idle',candidate,False),('resident-reload',candidate,True)]
progress=[]
for workload in ['ordered','letter','replaceable']:
 for block in range(8):
  for name,exe,reload in cases[block%4:]+cases[:block%4]:
   output=root/f'{workload}-{block}-{name}';before=os.getloadavg();start=time.time()
   command=[sys.executable,str(probe),str(exe),str(output),'--samples',str(64 if reload else 128),'--workload',workload]
   if reload:command+=['--reload']
   with (root/f'{output.name}.log').open('w') as log:
    result=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=120)
   progress.append(dict(workload=workload,block=block,case=name,exit=result.returncode,load_before=before,load_after=os.getloadavg(),start=start,end=time.time()))
   (root/'progress.json').write_text(json.dumps(progress,indent=2)+'\n')
   assert result.returncode==0,output
   print(workload,block,name,'PASS',flush=True)
print('PASS: 12288 completed requests across release, maintenance, resident idle and resident reload')
