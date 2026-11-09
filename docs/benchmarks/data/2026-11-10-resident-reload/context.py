#!/usr/bin/env python3
"""First config creation, damaged active pointer, retained-startup recovery."""
import json,os,subprocess,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(XDG_CONFIG_HOME=str(root/'config-home'),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null')
peers=[];result=dict(success=False,checks=[])
try:
    p=resident.Peer(exe,env,root,0);peers.append(p);initial=p.status();p.speech()
    cfg=root/'config-home'/'omnivox';cfg.mkdir(parents=True)
    resident.save(cfg/'config.json',dict(schema=4,speech=dict(punctuation=dict(all={'!':'fresh'}))))
    assert p.status()['configuration_file']==str(cfg/'config.json')
    changed=p.reload();assert changed['outcome']=='applied',changed;p.speech();result['checks'].append('creating first default configuration can reload')
    profile=next((root/'voices'/'profiles').iterdir());active=profile/'active.json';assert not active.exists();active.write_text('{damaged')
    resident.save(cfg/'config.json',dict(schema=4,speech=dict(punctuation=dict(all={'!':'independent'}))))
    changed=p.reload();assert changed['outcome']=='applied',changed;p.speech();result['checks'].append('reload is independent of damaged managed active pointer')
    p.send(b'OMNIVOX-LOCAL {"request_id":99,"command":"describe"}\n')
    row=p.wait(lambda r:r['kind']=='other' and r['value']['line'].startswith('OMNIVOX-LOCAL '))
    owner=json.loads(row['value']['line'].split(' ',1)[1]);result['retained_owner']=owner
    p.close();peers.clear()
    # A fresh capture is allowed to fail here; record the actual failure rather
    # than deleting state or inventing native cleanup evidence.
    fresh=subprocess.run([str(exe),'--voice-library-owner'],env=env,input=b'OMNIVOX-LOCAL {"request_id":100,"command":"describe"}\n',capture_output=True,timeout=10)
    (root/'fresh.stderr').write_bytes(fresh.stderr);(root/'fresh.stdout').write_bytes(fresh.stdout)
    assert b'error' in fresh.stdout,fresh.stdout
    restored=dict(env,OMNIVOX_OWNED_STARTUP=owner['startup'],OMNIVOX_OWNED_STARTUP_SHA256=owner['startup_sha256'])
    start=time.perf_counter_ns();p=resident.Peer(exe,restored,root,1);peers.append(p);speech=p.speech();result['first_usable_source_ms']=(speech['start_ns']-start)/1e6+speech['source_ms']
    status=p.status();assert status['revision']==0 and status['punctuation_sha256']==initial['punctuation_sha256']
    assert status['incarnation']!=initial['incarnation'];assert p.reload()['outcome']=='applied';p.speech()
    result['checks'].append('confirmed retired startup restores speech despite damaged pointer; optional punctuation reverts then reloads')
    result['success']=True
finally:
    for peer in peers:peer.close()
    resident.save(root/'result.json',result)
print(json.dumps(result,indent=2))
