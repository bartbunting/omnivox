#!/usr/bin/env python3
import argparse,sys,shutil
from pathlib import Path
sys.dont_write_bytecode=True
import probe
p=argparse.ArgumentParser();p.add_argument('output',type=Path);p.add_argument('--workload',default='ordered');a=p.parse_args()
r=Path('target/lightweight-investigation').resolve()
cases=[('release',Path('/home/bart/tmp/ov118-bench/release/omnivox'),False,0),('maintenance',r/'payload-maintenance/omnivox',False,0),('idle',r/'payload-candidate/omnivox',True,0),('reload',r/'payload-candidate/omnivox',True,100)]
a.output.mkdir(parents=True)
shutil.copy2(Path(__file__).with_name('probe.py'),a.output/'probe.py')
for block in range(8):
 for i in range(4):
  name,exe,enabled,operations=cases[(i+block)%4]
  result=probe.run(exe,a.output/f'{block}-{name}',64,enabled,False,operations,a.workload)
  print(block,name,'passed',len(result['samples']),flush=True)
