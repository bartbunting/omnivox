#!/usr/bin/env python3
"""Actual guarded owner and native stdio bridge; private Linux null output."""
import argparse, base64, ctypes, hashlib, json, os, select, signal, subprocess, sys, time
from pathlib import Path
sys.dont_write_bytecode=True
repo=next(p for p in Path(__file__).resolve().parents if (p/'Cargo.toml').is_file())
sys.path.insert(0,str(repo/'docs/benchmarks/data/2026-11-10-resident-reload'))
import resident
args=argparse.ArgumentParser();args.add_argument('exe',type=Path);args.add_argument('output',type=Path);a=args.parse_args()
exe=a.exe.resolve();root=a.output.resolve();root.mkdir(parents=True,exist_ok=False)
assert ctypes.CDLL(None).prctl(36,1,0,0,0)==0
results=[]
def ended(fd): return bool(select.select([fd],[],[],0)[0])
def signal_handle(fd,sig):
 if not ended(fd):signal.pidfd_send_signal(fd,sig)
def children(pid):
 try:return [int(x) for x in Path(f'/proc/{pid}/task/{pid}/children').read_text().split()]
 except FileNotFoundError:return []
def read(stream):
 assert select.select([stream],[],[],6)[0],'reply timeout'
 line=stream.readline();assert line,'lost authority';return json.loads(line.removeprefix(b'OMNIVOX-LOCAL '))
class Fixture:
 def __init__(self,label):
  self.root=root/label;self.root.mkdir();self.handles={};self.seq=0;self.log=[]
  self.env={k:os.environ[k] for k in ('HOME','PATH','LANG') if k in os.environ}
  self.env.update(OMNIVOX_CONFIG_DIR=str(self.root),OMNIVOX_VOICE_ROOT=str(self.root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null')
  resident.save(self.root/'config.json',resident.configuration(engine='flite'))
  self.err=(self.root/'stderr').open('wb')
  self.p=subprocess.Popen([exe,'--voice-library-owner','--guarded'],env=self.env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.err,bufsize=0)
  self.own=self.hold(self.p.pid)
  self.p.stdin.write(b'OMNIVOX-LOCAL {"command":"describe","request_id":1}\n');self.owner=read(self.p.stdout)
  assert self.owner['prepared'] and not self.owner['startup_error'],self.owner
  self.ep=self.owner['guard'];gs=children(self.p.pid);assert len(gs)==1,gs
  self.guard=gs[0];self.gf=self.hold(self.guard);assert not children(self.guard),'native start before client attachment'
  self.bridge=subprocess.Popen([exe,'--native-guard-control'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.err,bufsize=0)
  self.bridge.stdin.write(json.dumps(self.ep).encode()+b'\n');self.rpc('attach')
  self.p.stdin.write(b'OMNIVOX-LOCAL '+json.dumps(dict(command='start-guarded',worker=self.owner['worker'],request_id=2)).encode()+b'\n')
  assert read(self.p.stdout)['type']=='started'
  self.worker=children(self.guard)[0];self.wf=self.hold(self.worker)
  self.p.stdin.write(b'tts_say {Native helper initialization.}\n')
  deadline=time.monotonic()+5
  while not children(self.worker):
   assert time.monotonic()<deadline,'Flite helper absent';time.sleep(.01)
  self.helper=children(self.worker)[0];self.hf=self.hold(self.helper)
  assert 'flite' in str(Path(f'/proc/{self.helper}/exe').resolve())
  def links(pid):
   rows={}
   for path in Path(f'/proc/{pid}/fd').iterdir():
    try:rows[path.name]=os.readlink(path)
    except FileNotFoundError:pass
   return rows
  guard_fds=links(self.guard);worker_fds=links(self.worker)
  assert worker_fds['0'] not in guard_fds.values() and worker_fds['1'] not in guard_fds.values(), 'guard retained speech endpoints'
  assert not any('pidfd' in link for link in worker_fds.values()), 'worker inherited owner handle'
  (self.root/'descriptors.json').write_text(json.dumps(dict(guard=guard_fds,worker=worker_fds),indent=2))
  self.before=self.fingerprints()
 def hold(self,pid):
  assert Path(f'/proc/{pid}').exists();fd=os.pidfd_open(pid);self.handles[pid]=fd;return fd
 def fingerprints(self):
  return {str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in (self.root/'voices').rglob('*') if p.is_file()}
 def rpc(self,command):
  self.seq+=1;req=dict(command=command,request_id=self.seq,guard=self.ep['guard'],worker=self.ep['worker'],token=self.ep['token'])
  self.bridge.stdin.write(json.dumps(req).encode()+b'\n');value=read(self.bridge.stdout);self.log.append(value)
  assert value['request_id']==self.seq and value['guard']==self.ep['guard'] and value['worker']==self.ep['worker'] and not value['error'],value
  return value
 def proof(self):
  start=time.monotonic();r=self.rpc('retire')
  while not r['tree_retired']:
   assert time.monotonic()-start<6;r=self.rpc('status')
  assert ended(self.wf) and ended(self.hf),'false native proof'
  self.rpc('retire-owner');r=self.rpc('status')
  while not r['io_retired']:
   assert time.monotonic()-start<6;r=self.rpc('status')
  elapsed=1000*(time.monotonic()-start)
  self.log.append(dict(observation='owner-exited-at-io-proof',exited=ended(self.own)))
  assert select.select([self.own],[],[],2)[0], 'fixture owner exit did not follow retirement'
  return elapsed
 def close(self):
  self.bridge.stdin.close();self.bridge.wait(timeout=4);self.bridge.stdout.close()
  for fd in self.handles.values():signal_handle(fd,signal.SIGKILL)
  self.p.wait(timeout=4);self.p.stdin.close();self.p.stdout.close();self.err.close()
  for pid,fd in self.handles.items():
   assert select.select([fd],[],[],4)[0],f'fixture process survived {pid}'
   try:os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOHANG)
   except ChildProcessError:pass
   os.close(fd)
  (self.root/'voices/sessions').chmod(0o700)
  (self.root/'control.json').write_text(json.dumps(self.log,indent=2))
for label in ['owner-killed','owner-paused','blocked-input','blocked-output','helper-survives-worker','guard-lost']:
 f=None
 try:
  f=Fixture(label);row=dict(case=label)
  # Damage settings and host metadata only after native startup; retain bytes.
  (f.root/'config.json').write_text('{broken');(f.root/'voices/host.json').write_text('{broken')
  (f.root/'voices/sessions').chmod(0o500)
  before=f.fingerprints()
  if label=='owner-killed':signal_handle(f.own,signal.SIGKILL)
  elif label=='owner-paused':signal_handle(f.own,signal.SIGSTOP)
  elif label=='blocked-input':
   signal_handle(f.wf,signal.SIGSTOP);os.set_blocking(f.p.stdin.fileno(),False)
   payload=b'q {'+b'x'*16000+b'}\n';start=time.monotonic();written=0
   while time.monotonic()-start<.5:
    try:written+=os.write(f.p.stdin.fileno(),payload)
    except BlockingIOError:time.sleep(.005)
   waits=[p.read_text().strip() for p in Path(f'/proc/{f.p.pid}/task').glob('*/wchan')]
   assert any('sock_alloc_send' in w or w.endswith('pipe_write') for w in waits),waits
   row.update(written=written,waits=waits)
  elif label=='blocked-output':
   os.set_blocking(f.p.stdin.fileno(),False);payload=b'OMNIVOX-LOCAL {"command":"describe","request_id":100}\n';start=time.monotonic();written=0
   while time.monotonic()-start<.2:
    try:written+=os.write(f.p.stdin.fileno(),payload)
    except BlockingIOError:time.sleep(.005)
   waits=[p.read_text().strip() for p in Path(f'/proc/{f.p.pid}/task').glob('*/wchan')]
   assert any(w.endswith('pipe_write') for w in waits),waits
   row.update(written=written,waits=waits)
  elif label=='helper-survives-worker':
   signal_handle(f.own,signal.SIGSTOP);signal_handle(f.hf,signal.SIGSTOP);signal_handle(f.wf,signal.SIGKILL)
   assert select.select([f.wf],[],[],2)[0] and not ended(f.hf)
  elif label=='guard-lost':
   signal_handle(f.gf,signal.SIGKILL)
   try:f.rpc('retire')
   except (AssertionError,BrokenPipeError):row['replacement_blocked']=True
   else:raise AssertionError('guard loss became proof')
  if label!='guard-lost':row['proof_ms']=f.proof()
  assert before==f.fingerprints(),'cleanup modified management state'
  row['passed']=True;results.append(row);print(json.dumps(row),flush=True)
 finally:
  if f:f.close()
  (root/'result.json').write_text(json.dumps(results,indent=2))
