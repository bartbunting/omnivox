#!/usr/bin/env python3
"""Actual guarded owner and native stdio bridge; private Linux null output."""
import threading
import argparse, base64, ctypes, hashlib, json, os, select, signal, socket, subprocess, sys, time
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


def local(p, command, identity=1, **fields):
 p.stdin.write(b'OMNIVOX-LOCAL '+json.dumps(dict(command=command,request_id=identity,**fields)).encode()+b'\n');return read(p.stdout)
def wait_until(test, timeout=8):
 deadline=time.monotonic()+timeout
 while not test():
  assert time.monotonic()<deadline,'fault condition absent';time.sleep(.005)
def stopped(pid):
 try:return '\nState:\tT' in Path(f'/proc/{pid}/status').read_text()
 except FileNotFoundError:return False
def speak(p):
 value=dict(protocol_version=3,generation=812,dispatch_id=812,delivery_policy='ordered',spans=[dict(id=1,text='Speech after interrupted startup.')],actions=[])
 p.stdin.write(b'emacsvox_timeline '+resident.encoded(value)+b'\n');p.stdin.flush()
 started=False;deadline=time.monotonic()+5
 while time.monotonic()<deadline:
  assert select.select([p.stdout],[],[],5)[0]
  line=p.stdout.readline()
  if line.startswith(b'__EMACSVOX_MARKER__ '):
   v=json.loads(base64.b64decode(line.split()[1]));started|=v['type']=='utterance_started'
  if line.startswith(b'__EMACSVOX_TRACKED__ 812 '):
   assert started and b' completed' in line;return
 raise AssertionError('speech absent')
for mode in ['length','body','start']:
 healthy=Fixture('healthy-'+mode);held={};p=bridge=replacement=None
 try:
  folder=root/mode;folder.mkdir();err=(folder/'stderr').open('wb')
  env=dict(healthy.env,LD_PRELOAD=str(Path(__file__).with_name('partial.so')),OMNIVOX_GUARD_TEST_FAULT=mode)
  p=subprocess.Popen([exe,'--emergency-owner','--prepare','--guarded','--audio-output','null'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,bufsize=0)
  own=os.pidfd_open(p.pid);held[p.pid]=own;row=dict(case='partial-'+mode)
  if mode=='start':
   owner=local(p,'describe');ep=owner['guard'];g=children(p.pid)[0];gf=os.pidfd_open(g);held[g]=gf
   bridge=subprocess.Popen([exe,'--native-guard-control'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,bufsize=0)
   bridge.stdin.write(json.dumps(ep).encode()+b'\n');seq=0
   def rpc(command):
    global seq
    seq+=1;bridge.stdin.write(json.dumps(dict(command=command,request_id=seq,guard=ep['guard'],worker=ep['worker'],token=ep['token'])).encode()+b'\n')
    v=read(bridge.stdout);assert v['request_id']==seq and not v['error'];return v
   rpc('attach')
   p.stdin.write(b'OMNIVOX-LOCAL '+json.dumps(dict(command='start-emergency',request_id=2,worker=owner['worker'])).encode()+b'\n')
   wait_until(lambda:stopped(p.pid))
   workers=children(g);assert len(workers)==1;w=workers[0];wf=os.pidfd_open(w);held[w]=wf
   assert not children(w),'engine helper started before complete START'
   row['worker_gate_wait']=Path(f'/proc/{w}/wchan').read_text().strip()
   start=time.monotonic();v=rpc('retire')
   while not v['tree_retired']:v=rpc('status')
   assert ended(wf) and not v['io_retired'] and not ended(own), 'partial START lost I/O lease'
   row['tree_before_io']=v
   rpc('retire-owner');v=rpc('status')
   while not v['io_retired']:v=rpc('status')
   row.update(proof=v,retire_ms=1000*(time.monotonic()-start))
   assert select.select([own],[],[],2)[0]
  else:
   wait_until(lambda:stopped(p.pid));gs=children(p.pid);assert len(gs)==1;g=gs[0];gf=os.pidfd_open(g);held[g]=gf
   assert not children(g),'native child before bootstrap and client attach'
   start=time.monotonic()
   if mode=='length':signal_handle(own,signal.SIGKILL)
   assert select.select([gf],[],[],8)[0], 'partial bootstrap guard failed to retire'
   row['guard_exit_ms']=1000*(time.monotonic()-start)
   assert not children(g)
   signal_handle(own,signal.SIGKILL)
  speak(healthy.p);row['healthy_peer_speech']=True
  if mode=='start':
   clean={k:v for k,v in env.items() if k not in ('LD_PRELOAD','OMNIVOX_GUARD_TEST_FAULT')}
   replacement=subprocess.Popen([exe,'--emergency-owner','--audio-output','null'],env=clean,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=err,bufsize=0)
   speak(replacement);replacement.stdin.close();assert replacement.wait(timeout=6)==0;replacement.stdout.close();replacement=None
   row['minimal_replacement_speech']=True
  row['passed']=True;results.append(row);print(json.dumps(row),flush=True)
 finally:
  if bridge:bridge.stdin.close();bridge.wait(timeout=4);bridge.stdout.close()
  for fd in held.values():signal_handle(fd,signal.SIGKILL)
  if p:p.wait(timeout=4);p.stdin.close();p.stdout.close()
  if replacement:replacement.kill();replacement.wait(timeout=4)
  for pid,fd in held.items():
   assert select.select([fd],[],[],4)[0]
   try:os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOHANG)
   except ChildProcessError:pass
   os.close(fd)
  healthy.close();err.close()
  (root/'result.json').write_text(json.dumps(results,indent=2))
