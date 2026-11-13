#!/usr/bin/env python3
"""Matched guarded/direct source timing during per-worker reload; no acoustic claim."""
import concurrent.futures, hashlib, json, os, select, statistics, subprocess, sys, time, threading
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
results=[]
for engine in ['espeak','flite']:
 for repeat in range(2):
  for guarded in ([False] if os.environ.get("ONLY_DIRECT") else ([False,True] if repeat==0 else [True,False])):
   folder=root/f'{engine}-{repeat}-{guarded}';folder.mkdir()
   env={k:os.environ[k] for k in ('HOME','PATH','LANG') if k in os.environ}
   env.update(OMNIVOX_CONFIG_DIR=str(folder),OMNIVOX_VOICE_ROOT=str(folder/'voices'),OMNIVOX_ENGINE=engine,OMNIVOX_AUDIO_OUTPUT='null')
   resident.save(folder/'config.json',resident.configuration(engine=engine))
   peers=[];row=dict(engine=engine,guarded=guarded,repeat=repeat,samples=[],reloads=[],protocol_ms=[])
   try:
    start=time.perf_counter_ns()
    for lane in ['main','notify']:
     p=Peer([str(exe),'--voice-library-owner']+(['--guarded'] if guarded else []),env,folder,lane,engine);peers.append(p);p.register();p.speech('A','letter')
    row['two_ready_ms']=(time.perf_counter_ns()-start)/1e6
    p,notice=peers
    for _ in range(50):
     start=time.perf_counter_ns();p.rpc('capabilities');row['protocol_ms'].append((time.perf_counter_ns()-start)/1e6)
    with concurrent.futures.ThreadPoolExecutor(2) as pool:
     for n in range(20):
      resident.save(folder/'config.json',resident.configuration(str(n),engine))
      p.reload_submitted=threading.Event()
      reload=pool.submit(p.reload);notify=pool.submit(notice.speech,'N','letter')
      assert p.reload_submitted.wait(6), 'reload was not transmitted before foreground input'
      row['samples'].append(p.speech('A','letter'));row['reloads'].append(reload.result());notify.result()
      assert row['reloads'][-1]['outcome']=='applied'
    row['source_median_ms']=statistics.median(s['source_ms'] for s in row['samples'])
    row['protocol_median_ms']=statistics.median(row['protocol_ms'])
    print(json.dumps({k:v for k,v in row.items() if k not in ['samples','reloads','protocol_ms']}),flush=True)
    results.append(row)
   finally:
    for p in peers:p.close()
    (root/'result.json').write_text(json.dumps(dict(sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),runs=results),indent=2))
