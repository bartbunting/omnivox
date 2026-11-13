#!/usr/bin/env python3
"""File isolation and helper-retention checks; no installed paths are written."""
import copy,json,os,sys
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True);exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex())
cfg=probe.configuration();cfg['engine_overrides'].pop('flite');probe.save(root/'config.json',cfg)
owners=[];modes={};result=dict(success=False)
def resources(pid):
 rows={}
 for child in Path(f'/proc/{pid}/task/{pid}/children').read_text().split():
  p=int(child);stat=Path(f'/proc/{p}/stat').read_text().split()
  rows[child]=dict(start=stat[21],rss_pages=int(stat[23]),fds=len(list(Path(f'/proc/{p}/fd').iterdir())))
  rows.update(resources(p))
 return rows
try:
 for _ in range(2):owners.append(probe.Owner(exe,env))
 for o in owners:o.speech()
 a,_=probe.prepare(exe,env);b=copy.deepcopy(a);b['punctuation']['profiles']['probe']['table']['!']='captured'
 # A later invalid file must not change the captured candidate used by either lane.
 probe.save(root/'config.json',dict(schema=999))
 for o in owners:assert o.request(b)['outcome']=='applied';o.speech()
 result['captured_despite_later_invalid_file']=True
 # Existing speech and readable captured candidates do not need history writes.
 paths=[root]+[p for p in root.rglob('*') if p.is_dir()]
 for p in paths:modes[p]=p.stat().st_mode & 0o777;p.chmod(0o500)
 try:(root/'write-must-fail').write_text('test');raise AssertionError('fixture not read-only')
 except PermissionError:pass
 for o in owners:assert o.request(a)['outcome']=='applied';o.speech()
 result['read_only_management_state']=True
 before=[resources(o.p.pid) for o in owners]
 for i in range(500):
  o=owners[i%2];r=o.request(b if (i//2)%2==0 else a);assert r['outcome']=='applied'
 after=[resources(o.p.pid) for o in owners]
 for x,y in zip(before,after):
  assert x.keys()==y.keys()
  for pid in x:assert (x[pid]['start'],x[pid]['fds'])==(y[pid]['start'],y[pid]['fds'])
 result.update(before=before,after=after,success=True)
finally:
 for p,mode in modes.items():p.chmod(mode)
 for i,o in enumerate(owners):o.close();(root/f'owner-{i}.log').write_bytes(o.err)
 probe.save(root/'result.json',result)
print('PASS frozen candidate, denied writes, 500 reloads retaining real Flite helpers')
