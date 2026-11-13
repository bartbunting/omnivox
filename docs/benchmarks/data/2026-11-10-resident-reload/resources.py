#!/usr/bin/env python3
"""Linux resource bounds across repeated resident reloads; never touches live speech."""
import json,os,sys
from pathlib import Path
sys.dont_write_bytecode=True
import resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null')
resident.save(root/'config.json',resident.configuration(engine='flite'));peer=resident.Peer(exe,env,root,0);rows=[]
def resources():
    children=Path(f'/proc/{peer.p.pid}/task/{peer.p.pid}/children').read_text().split();assert len(children)==1,children
    pid=children[0];status=Path(f'/proc/{pid}/status').read_text()
    return dict(pid=int(pid),fds=len(list(Path(f'/proc/{pid}/fd').iterdir())),threads=len(list(Path(f'/proc/{pid}/task').iterdir())),rss_kib=int(next(line for line in status.splitlines() if line.startswith('VmRSS:')).split()[1]),management_files=len([p for p in (root/'voices').rglob('*') if p.is_file()]))
try:
    peer.speech();before=resources()
    for n in range(500):
        resident.save(root/'config.json',resident.configuration(str(n),engine='flite'))
        row=peer.reload();assert row['outcome']=='applied';rows.append(row)
        if n%10==0:peer.speech()
    after=resources();assert all(before[key]==after[key] for key in ['pid','fds','threads','management_files']),(before,after)
    assert peer.status()['revision']==500
finally:
    peer.close();resident.save(root/'result.json',dict(success=len(rows)==500,before=before,after=locals().get('after'),operations=rows))
print(json.dumps(dict(success=True,before=before,after=after),indent=2))
