#!/usr/bin/env python3
"""Matched guarded/direct source timing during per-worker reload; no acoustic claim."""
import signal, concurrent.futures, hashlib, json, os, select, statistics, subprocess, sys, time, threading
from pathlib import Path
sys.dont_write_bytecode=True
repo=next(p for p in Path(__file__).resolve().parents if (p/'Cargo.toml').is_file())
sys.path.insert(0,str(repo/'docs/benchmarks/data/2026-11-10-emergency-speech'))
import probe as emergency
resident=emergency.resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True,exist_ok=False)
class Peer(emergency.Peer):
 def rpc(self,kind,**fields):
  if kind != 'reload_punctuation_v1':return super().rpc(kind,**fields)
  with self.write_lock:
   self.control+=1;identity=self.control
   self.p.stdin.write(b'omnivox_control '+resident.encoded(dict(protocol_version=1,request_id=identity,type=kind,**fields))+b'\n');self.p.stdin.flush()
  self.reload_submitted.set()
  return self.wait(lambda r:r['kind']=='control' and r['value'].get('request_id')==identity)['value']
 def local(self,command,**fields):
  value=super().local(command,**fields)
  if command=='describe' and value.get('guard') and not hasattr(self,'bridge'):
   ep=value['guard'];self.ep=ep;self.guard_seq=0
   self.bridge=subprocess.Popen([exe,'--native-guard-control'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.stderr,bufsize=0)
   self.bridge.stdin.write(json.dumps(ep).encode()+b'\n');self.guard('attach')
   assert super().local('start-guarded',worker=value['worker'])['type']=='started'
  return value
 def guard(self,command):
  self.guard_seq+=1;ep=self.ep
  self.bridge.stdin.write(json.dumps(dict(command=command,guard=ep['guard'],worker=ep['worker'],token=ep['token'],request_id=self.guard_seq)).encode()+b'\n')
  assert select.select([self.bridge.stdout],[],[],6)[0];r=json.loads(self.bridge.stdout.readline());assert not r['error'];return r
 def close(self):
  if hasattr(self,'bridge'):
   self.retire();self.bridge.stdin.close();self.bridge.wait(timeout=5);self.bridge.stdout.close()
  return super().close()

def submit(p, text, policy='ordered'):
 p.dispatch+=1;identity=p.dispatch
 value=dict(protocol_version=3,generation=identity,dispatch_id=identity,delivery_policy=policy,spans=[dict(id=1,text=text)],actions=[])
 if policy=='replaceable':value['replacement_key']='interaction'
 start=time.perf_counter_ns();p.send(b'emacsvox_timeline '+resident.encoded(value)+b'\n');return identity,start
def started(p,identity):
 return p.wait(lambda r:r['kind']=='marker' and r['value']['dispatch_id']==identity and r['value']['type']=='utterance_started')
def terminal(p,identity):return p.wait(lambda r:r['kind']=='terminal' and r['value']['dispatch_id']==identity)
def children(pid):return [int(x) for x in Path(f'/proc/{pid}/task/{pid}/children').read_text().split()]
results=[]
for engine in ['espeak','flite','winrt','eloquence','dectalk','rutts','tgspeechbox','rhvoice']:
 for guarded in [False,True]:
  folder=root/f'{engine}-{guarded}';folder.mkdir();peers=[];fd=None
  env={k:os.environ[k] for k in ('HOME','PATH','LANG','WSL_INTEROP') if k in os.environ}
  env.update(OMNIVOX_CONFIG_DIR=str(folder),OMNIVOX_VOICE_ROOT=str(folder/'voices'),OMNIVOX_ENGINE=engine,OMNIVOX_AUDIO_OUTPUT='null')
  env['ESPEAK_NG_DATA']=os.environ['OMNIVOX_TEST_WINDOWS_DATA']
  env['WSLENV']='OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA'
  env.update(OMNIVOX_RHVOICE_LIBRARY=os.environ['OMNIVOX_TEST_RHVOICE_LIBRARY'],OMNIVOX_RHVOICE_DATA=os.environ['OMNIVOX_TEST_RHVOICE_DATA']);env['WSLENV']+=':OMNIVOX_RHVOICE_LIBRARY:OMNIVOX_RHVOICE_DATA'
  resident.save(folder/'config.json',dict(resident.configuration(engine=engine),engine_overrides={k:dict(enabled=False) for k in ['macos','piper','mbrola']}))
  row=dict(engine=engine,guarded=guarded,stop=[],replace=[],notifications=[])
  try:
   for lane in ['main','notify']:
    p=Peer([str(exe),'--voice-library-owner']+(['--guarded'] if guarded else []),env,folder,lane,engine);peers.append(p);p.register();p.speech('Б' if engine=='rutts' else 'A','letter')
   p,notice=peers
   with concurrent.futures.ThreadPoolExecutor(2) as pool:
    for mode in ['stop','replace']:
     for n in range(20):
      identity,start=submit(p,('Это длинное предложение прерывается при изменении настроек. ' if engine=='rutts' else 'This sustained sentence is interrupted while preferences change. ')*12,'replaceable' if mode=='replace' else 'ordered')
      first=started(p,identity);time.sleep(.025)
      resident.save(folder/'config.json',dict(resident.configuration(f'{mode}-{n}',engine),engine_overrides={k:dict(enabled=False) for k in ['macos','piper','mbrola']}));p.reload_submitted=threading.Event()
      change=pool.submit(p.reload);notification=pool.submit(notice.speech,'Б' if engine=='rutts' else 'N','letter');assert p.reload_submitted.wait(6)
      interrupt=time.perf_counter_ns()
      if mode=='stop':p.send(b's\n')
      new,start_new=submit(p,'Новый.' if engine=='rutts' else 'New.', 'replaceable' if mode=='replace' else 'ordered')
      source=started(p,new);old=terminal(p,identity);new_end=terminal(p,new)
      assert old['value']['outcome']!='completed', old
      assert new_end['value']['outcome']=='completed',new_end
      assert source['value']['engine_id']==engine,source
      assert change.result()['outcome']=='applied'
      row['notifications'].append(notification.result())
      row[mode].append(dict(old_outcome=old['value']['outcome'],old_source_ms=(first['ns']-start)/1e6,cancellation_ms=(old['ns']-interrupt)/1e6,replacement_source_ms=(source['ns']-start_new)/1e6))
   row['passed']=True;results.append(row);print(json.dumps({k:v for k,v in row.items() if not isinstance(v,list)}),flush=True)
  finally:
   if fd is not None:signal.pidfd_send_signal(fd,signal.SIGCONT);os.close(fd)
   for p in peers:p.close()
   (root/'result.json').write_text(json.dumps(dict(sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),runs=results),indent=2))
