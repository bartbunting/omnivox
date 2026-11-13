#!/usr/bin/env python3
"""Bounded independently paced replacement, Stop and notification probe."""
import base64,concurrent.futures,copy,json,os,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex())
probe.save(root/'config.json',probe.configuration())
owners=[];result=dict(success=False,events=[],sent=[],operations=[],notifications=[])
try:
 for _ in range(2):owners.append(probe.Owner(exe,env))
 o,notify=owners
 a,_=probe.prepare(exe,env);b=copy.deepcopy(a);b['punctuation']['profiles']['probe']['table']['!']='updated'
 def reload():
  for i in range(300):
   r=o.request(b if i%2==0 else a);assert r['outcome']=='applied',r
   result['operations'].append(r);time.sleep(.002)
 def notifications():
  for _ in range(40):result['notifications'].append(notify.speech('Notification'));time.sleep(.01)
 def submit():
  start=time.monotonic()
  for i in range(1,201):
   text=('A long continuing passage! '*500 if i==1 else 'A')
   timeline=dict(protocol_version=3,generation=i,dispatch_id=i,delivery_policy='replaceable',replacement_key='navigation',spans=[dict(id=1,text=text)],actions=[])
   if i%20==0:
    result['sent'].append(dict(kind='stop',ns=time.perf_counter_ns()));o.send(b's\n')
   result['sent'].append(dict(kind='speech',id=i,ns=time.perf_counter_ns()))
   o.send(b'emacsvox_timeline '+base64.b64encode(probe.record(timeline))+b'\n')
   time.sleep(max(0,start+i*.005-time.monotonic()))
 with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
  tasks=[pool.submit(f) for f in (reload,notifications,submit)]
  terminal={}
  while len(terminal)<200:
   line=o.line();result['events'].append(dict(ns=time.perf_counter_ns(),line=line.decode()))
   if line.startswith(b'__EMACSVOX_TRACKED__ '):
    _,n,status=line.split();n=int(n);assert n not in terminal;assert status in (b'completed',b'cancelled');terminal[n]=status.decode()
  for f in tasks:f.result()
 assert terminal[200]=='completed'
 assert any(v=='cancelled' for v in terminal.values())
 o.seq=200;o.speech();result['terminals']=terminal;result['success']=True
finally:
 for i,o in enumerate(owners):o.close();(root/f'owner-{i}.log').write_bytes(o.err)
 probe.save(root/'result.json',result)
print('PASS 200 independently paced replacements, 10 Stops, 40 notifications and 300 reloads')
