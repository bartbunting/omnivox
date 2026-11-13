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

def speech(f):
 identity=9000
 value=dict(protocol_version=3,generation=identity,dispatch_id=identity,delivery_policy='ordered',spans=[dict(id=1,text='A')],actions=[])
 f.p.stdin.write(b'emacsvox_timeline '+resident.encoded(value)+b'\n')
 deadline=time.monotonic()+5;first=False;done=False
 while time.monotonic()<deadline:
  if not select.select([f.p.stdout],[],[],.05)[0]:continue
  line=f.p.stdout.readline()
  if line.startswith(b'__EMACSVOX_MARKER__ '):
   v=json.loads(base64.b64decode(line.split()[1]));first|=v['dispatch_id']==identity and v['type']=='utterance_started' and v['engine_id']=='flite'
  if line.startswith(b'__EMACSVOX_TRACKED__ '):
   v=line.split();done=int(v[1])==identity and v[2]==b'completed'
  if first and done:return
 raise AssertionError('healthy speech did not complete')

def connect(f):
 f.bridge.stdin.close();f.bridge.wait(timeout=3);f.bridge.stdout.close()
 channel=socket.socket(socket.AF_UNIX);channel.settimeout(3);channel.connect('\0'+f.ep['address']);buf=channel.makefile('rwb',buffering=0)
 def request(command,identity):
  buf.write(json.dumps(dict(command=command,request_id=identity,guard=f.ep['guard'],worker=f.ep['worker'],token=f.ep['token'])).encode()+b'\n')
  value=read(buf);assert value['request_id']==identity and not value['error'],value;return value
 request('attach',10000)
 return channel,buf,request

def process_state(pid):
 state={}
 for line in Path(f'/proc/{pid}/status').read_text().splitlines():
  k,_,v=line.partition(':')
  if k in ('voluntary_ctxt_switches','nonvoluntary_ctxt_switches','VmRSS'):state[k]=v.strip()
 state['wait']=Path(f'/proc/{pid}/wchan').read_text().strip()
 return state

for label in ['idle-wakeups','client-disconnect','partial-control','held-status']:
 f=None;channel=None;buf=None
 try:
  f=Fixture(label);row=dict(case=label)
  if label=='idle-wakeups':
   f.rpc('status');before=process_state(f.guard);start=time.monotonic();time.sleep(1);after=process_state(f.guard)
   row.update(before=before,after=after,elapsed=time.monotonic()-start)
   speech(f)
  else:
   channel,buf,request=connect(f)
   if label=='client-disconnect':
    speech(f);row['speech_after_disconnect']=True
    r=request('status',10001);assert r['started'] and not r['tree_retired']
   elif label=='partial-control':
    buf.write(b'{"request_id":');start=time.monotonic();channel.settimeout(8)
    assert buf.read(1)==b'', 'partial control did not expire'
    row['expiry_ms']=1000*(time.monotonic()-start);buf.close();channel.close()
    speech(f);row['speech_after_timeout']=True
    channel,buf,request=connect(f);assert request('status',10001)['started']
   else:
    # Fill the actual guard reply path while withholding reads. Keep the
    # original guard and worker identities; this channel owns no speech.
    channel.setblocking(False)
    payload=b''.join(json.dumps(dict(command='status',request_id=i,guard=f.ep['guard'],worker=f.ep['worker'])).encode()+b'\n' for i in range(10001,10801))
    remaining=payload;deadline=time.monotonic()+2
    while remaining and time.monotonic()<deadline:
     try:remaining=remaining[channel.send(remaining):]
     except BlockingIOError:time.sleep(.002)
    time.sleep(.2)
    rows=subprocess.check_output(['ss','-xanpm'],text=True).splitlines();selected=[]
    for i,line in enumerate(rows):
     if f'pid={f.guard},' in line:
      selected.extend(rows[i:i+2])
    row.update(queued_bytes=len(payload)-len(remaining),socket_observations=selected,guard=process_state(f.guard))
    assert any('w' in line for line in selected), 'no native socket queue observation'
    # Inspect the actual poll interest in this fixture-owned descendant. This
    # read-only observation is not process signalling or cleanup authority.
    call=Path(f'/proc/{f.guard}/syscall').read_text().split()
    row['syscall']=call
    if call[0]=='7':
     class IOV(ctypes.Structure):_fields_=[('base',ctypes.c_void_p),('length',ctypes.c_size_t)]
     class Poll(ctypes.Structure):_fields_=[('fd',ctypes.c_int),('events',ctypes.c_short),('revents',ctypes.c_short)]
     count=int(call[2],16);assert 0<count<=5
     storage=(Poll*count)();local=IOV(ctypes.addressof(storage),ctypes.sizeof(storage));remote=IOV(int(call[1],16),ctypes.sizeof(storage))
     copied=ctypes.CDLL(None,use_errno=True).process_vm_readv(f.guard,ctypes.byref(local),1,ctypes.byref(remote),1,0)
     assert copied==ctypes.sizeof(storage),(copied,ctypes.get_errno())
     row['poll_interests']=[dict(fd=v.fd,events=v.events) for v in storage]
     assert any(v.events & 4 for v in storage), 'guard was not waiting for writable status output'
    else:
     # The preserved polling variant sleeps between nonblocking attempts.
     assert 'sleep' in row['guard']['wait'],row
    signal_handle(f.own,signal.SIGKILL);start=time.monotonic()
    assert select.select([f.wf],[],[],2)[0] and select.select([f.hf],[],[],2)[0]
    row['native_exit_while_status_held_ms']=1000*(time.monotonic()-start)
    channel.settimeout(10)
    # Drain replies concurrently with resuming requests: both directions are
    # bounded and can legitimately fill while the receiver withholds reads.
    tail=remaining+json.dumps(dict(command='status',request_id=12000,guard=f.ep['guard'],worker=f.ep['worker'])).encode()+b'\n'
    writer=threading.Thread(target=channel.sendall,args=(tail,));writer.start()
    while True:
     v=read(buf)
     assert v['guard']==f.ep['guard'] and v['worker']==f.ep['worker']
     if v['request_id']==12000:
      assert v['tree_retired'] and v['io_retired'];row['proof']=v;break
    writer.join(timeout=10);assert not writer.is_alive()
  row['passed']=True;results.append(row);print(json.dumps(row),flush=True)
 finally:
  if buf:buf.close()
  if channel:channel.close()
  if f:f.close()
  (root/'result.json').write_text(json.dumps(results,indent=2))
