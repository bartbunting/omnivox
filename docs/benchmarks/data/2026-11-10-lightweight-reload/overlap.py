#!/usr/bin/env python3
"""Counterbalanced candidate-only runs with timestamped management intervals."""
import shutil,sys
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[1]).resolve();root.mkdir(parents=True)
shutil.copy2(Path(__file__).with_name('probe.py'),root/'probe.py')
exe=Path('target/lightweight-investigation/payload-candidate/omnivox').resolve()
for block in range(5):
 for reloads in ([0,1000] if block%2==0 else [1000,0]):
  name='reload' if reloads else 'idle'
  result=probe.run(exe,root/f'{block}-{name}',100,True,False,reloads)
  print(block,name,len(result['samples']),flush=True)
