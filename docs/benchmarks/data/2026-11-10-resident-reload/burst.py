#!/usr/bin/env python3
"""Rapid replacement/Stop and notifications while policies change independently."""
import concurrent.futures,json,os,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null')
resident.save(root/'config.json',resident.configuration(engine='flite'));peers=[];result=dict(success=False,operations=[],notifications=[],terminals=[])
try:
    peers=[resident.Peer(exe,env,root,i) for i in range(2)]
    for peer in peers:peer.register();peer.speech()
    def reloads():
        for n in range(25):
            resident.save(root/'config.json',resident.configuration(str(n),engine='flite'))
            row=peers[0].reload();assert row['outcome']=='applied';result['operations'].append(row);time.sleep(.01)
    def notifications():
        for n in range(15):result['notifications'].append(peers[1].speech('Notice'))
    p=peers[0];identities=[]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        futures=[pool.submit(reloads),pool.submit(notifications)]
        for i in range(200):
            p.dispatch+=1;identity=p.dispatch;identities.append(identity)
            value=dict(protocol_version=4,generation=identity,dispatch_id=identity,delivery_policy='ordered',registry_generation=1,actions=[],spans=[dict(mode='layered',span=dict(id=1,text=chr(65+i%26),logical_voice_id='probe',context={},placement=dict(pan=None)))])
            p.send(b'emacsvox_letter '+resident.encoded(value)+b'\n')
            if i%13==0:p.send(b's\n')
            time.sleep(.002)
        for future in futures:future.result()
    p.send(b's\n')
    for identity in identities:
        terminal=p.wait(lambda r:r['kind']=='terminal' and r['value']['dispatch_id']==identity)
        assert terminal['value']['outcome'] in ('completed','cancelled'),terminal
        result['terminals'].append(terminal)
    result['recovery']=p.speech('Recovered')
    assert any(row['value']['outcome']=='cancelled' for row in result['terminals'])
    assert not any(row['kind']=='terminal' and row['value']['dispatch_id'] in identities for row in p.rows),'duplicate terminal'
    result['success']=True
finally:
    for peer in peers:peer.close()
    resident.save(root/'result.json',result)
print('PASS: 200 rapid characters, repeated Stop, 25 reloads, 15 concurrent notifications and fresh speech')
