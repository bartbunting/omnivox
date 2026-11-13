#!/usr/bin/env python3
import base64,concurrent.futures,copy,json,os,socket,struct,subprocess,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex())
probe.save(root/'config.json',probe.configuration())
checks=[]; owners=[]
def check(name): checks.append(name);print(name,flush=True);probe.save(root/'checks.json',checks)
def children(pid):
    result={}
    try:
        for child in Path(f'/proc/{pid}/task/{pid}/children').read_text().split():
            p=int(child); result[p]=Path(f'/proc/{p}/stat').read_text().split()[21];result.update(children(p))
    except FileNotFoundError:pass
    return result
try:
 for _ in range(2):owners.append(probe.Owner(exe,env))
 o=owners[0]; initial=[children(x.p.pid) for x in owners];fds=[len(list(Path(f'/proc/{list(k)[0]}/fd').iterdir())) for k in initial]
 a,_=probe.prepare(exe,env);b=copy.deepcopy(a);b['punctuation']['profiles']['probe']['table']['!']='new'
 rev=o.endpoint['state']['policy'];assert o.request(b)['outcome']=='applied';assert o.request(a)['outcome']=='applied';assert o.request(b,expected=rev)['outcome']=='stale';check('ABA stale retry rejected')
 o.send(b'tts_set_punctuation_profile probe\n');o.speech()
 removed=copy.deepcopy(a);removed['punctuation']['profiles']={}
 assert o.request(removed)['outcome']=='rejected';o.speech();check('selected profile removal preserves speech')
 # Drop a reply after sending a complete operation. Observe actual policy later.
 state=o.endpoint['state'];address=o.endpoint['address'].rsplit(':',1)
 value=dict(token=o.token,incarnation=state['incarnation'],expected=state['policy'],candidate=b);raw=probe.record(value)
 with socket.create_connection((address[0],int(address[1]))) as s:s.sendall(struct.pack('>I',len(raw))+raw)
 assert o.request()['state']['policy']!=state['policy'];assert o.request(b,expected=state['policy'])['outcome']=='unchanged';check('lost reply observes actual installation')
 # Request installation while one logical request is only partially received.
 o.seq+=1; n=o.seq; document=probe.record(dict(protocol_version=3,generation=n,dispatch_id=n,delivery_policy='ordered',spans=[dict(id=1,text='hello!')],actions=[])); wire=base64.b64encode(document);mid=(len(wire)//8)*4
 def part(i,bits):return f'emacsvox_timeline_part 3 {n} {n} {i} 2 {len(document)} '.encode()+bits+b'\n'
 with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
    o.send(part(0,wire[:mid]));future=pool.submit(o.request,a);time.sleep(.03);o.send(part(1,wire[mid:]))
    events=[]
    while True:
      line=o.line();events.append(line.decode())
      if line.startswith(b'__EMACSVOX_TRACKED__ '):assert line.split()[1:]==[str(n).encode(),b'completed'];break
    assert future.result()['outcome']=='applied'
 probe.save(root/'multipart-events.json',events);check('multipart completes across reload wake')
 # Oversized management framing is rejected without affecting speech.
 with socket.create_connection((address[0],int(address[1]))) as s:
    s.sendall(struct.pack('>I',2**32-1));assert b'rejected' in s.recv(1000)
 o.speech();check('oversized management input preserves speech')
 for i in range(500):assert o.request(b if i%2==0 else a)['outcome']=='applied'
 after=[children(x.p.pid) for x in owners];assert initial==after
 fds_after=[len(list(Path(f'/proc/{list(k)[0]}/fd').iterdir())) for k in after]
 assert fds==fds_after,(fds,fds_after)
 probe.save(root/'resources.json',dict(before=initial,after=after,fds_before=fds,fds_after=fds_after));check('500 reloads retain native identities and descriptor counts')
 # Retained-startup recovery works despite malformed current files.
 o.send(b'OMNIVOX-LOCAL '+probe.record(dict(request_id=99,command='describe'))+b'\n')
 owner=json.loads(o.line().split(b' ',1)[1]);o.close();owners.pop(0)
 probe.save(root/'config.json',dict(schema=999))
 recovery_env=dict(env,OMNIVOX_OWNED_STARTUP=owner['startup'],OMNIVOX_OWNED_STARTUP_SHA256=owner['startup_sha256'])
 recovered=probe.Owner(exe,recovery_env);owners.insert(0,recovered);recovered.speech();owners[1].speech();check('retained startup restores speech despite malformed current file')
finally:
 for i,o in enumerate(owners):o.close();(root/f'owner-{i}.log').write_bytes(o.err)
probe.save(root/'result.json',dict(success=True,checks=checks))
