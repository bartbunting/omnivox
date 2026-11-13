#!/usr/bin/env python3
"""WSL client to isolated native Windows owners; no device audio."""
import base64,copy,json,os,subprocess,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import probe
runtime=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
win_runtime=(runtime/'windows-runtime.path').read_text().strip()
unix_runtime=Path(subprocess.check_output(['wslpath','-u',win_runtime],text=True).strip())
exe=unix_runtime/'omnivox.exe'
env={k:os.environ[k] for k in ['PATH','HOME','LANG','WSL_INTEROP'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex(),ESPEAK_NG_DATA=(runtime/'espeak-ng-data.path').read_text().strip())
env['WSLENV']='OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN:ESPEAK_NG_DATA'
cfg=probe.configuration();cfg['engine_overrides'].pop('flite');probe.save(root/'config.json',cfg)
owners=[];result=dict(success=False,mode='WSL direct loopback',samples=[],operations=[])
try:
 for _ in range(2):
  o=probe.Owner.__new__(probe.Owner);owners.append(o);probe.Owner.__init__(o,exe,env)
 for o in owners:o.speech()
 cfg['speech']['punctuation_profiles']['probe']['overrides']['!']='windows updated';probe.save(root/'config.json',cfg)
 a,prep=probe.prepare(exe,env);result['prepare_ms']=prep
 # Test actual cross-OS connectivity before choosing a functional-only bridge.
 try:owners[0].request()
 except OSError as error:
  result['direct_loopback_error']=str(error);result['mode']='WSL speech; PowerShell native management bridge (functional only)'
  def native_request(self,candidate=None,expected=None,**override):
   state=self.endpoint['state'];value=dict(token=self.token,incarnation=state['incarnation'],expected=expected or state['policy'],candidate=candidate);value.update(override)
   address=self.endpoint['address'].rsplit(':',1)
   script='''$ErrorActionPreference='Stop'
$bytes=[Text.Encoding]::UTF8.GetBytes([Console]::In.ReadToEnd())
$client=New-Object Net.Sockets.TcpClient
try {
 $task=$client.ConnectAsync('127.0.0.1',PORT)
 if (!$task.Wait(5000)) {throw 'connect timeout'}
 $stream=$client.GetStream();$stream.ReadTimeout=5000;$stream.WriteTimeout=5000
 $header=[BitConverter]::GetBytes([Net.IPAddress]::HostToNetworkOrder([int]$bytes.Length))
 $stream.Write($header,0,4);$stream.Write($bytes,0,$bytes.Length)
 $reader=New-Object IO.StreamReader($stream)
 [Console]::Out.WriteLine($reader.ReadLine())
} finally {$client.Dispose()}
'''.replace('PORT',str(int(address[1])))
   p=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-EncodedCommand',base64.b64encode(script.encode('utf-16le')).decode()],input=probe.record(value),capture_output=True,timeout=15)
   assert p.returncode==0,p.stderr.decode();r=json.loads(p.stdout)
   if 'state' in r:self.endpoint['state']=r['state']
   return r
  for o in owners:o.request=native_request.__get__(o,probe.Owner)
 result['management_status']=owners[0].request();assert result['management_status']['outcome']=='observed',result['management_status']
 o=owners[0];o.send(b'OMNIVOX-LOCAL '+probe.record(dict(request_id=99,command='describe'))+b'\n');description=json.loads(o.line().split(b' ',1)[1])
 retained=Path(subprocess.check_output(['wslpath','-u',description['startup']],text=True).strip());previous=json.loads(retained.read_text())['engines']
 def differences(x,y,path=''):
  if isinstance(x,dict) and isinstance(y,dict):return [v for k in x.keys()|y.keys() for v in differences(x.get(k),y.get(k),path+'/'+k)]
  if isinstance(x,list) and isinstance(y,list) and len(x)==len(y):return [v for i,(u,w) in enumerate(zip(x,y)) for v in differences(u,w,path+'/'+str(i))]
  return [path] if x!=y else []
 result['candidate_difference_paths']=differences(previous,a)
 assert set(result['candidate_difference_paths']) <= {'/activation_id','/punctuation/profiles/probe/table/!'},result['candidate_difference_paths']
 b=copy.deepcopy(a);b['punctuation']['profiles']['probe']['table']['!']='windows alternate'
 for i in range(10):
  for lane,o in enumerate(owners):
   r=o.request(a if i%2==0 else b);assert r['outcome']=='applied',r;result['operations'].append(dict(lane=lane,**r));result['samples'].append(dict(lane=lane,**o.speech()))
 result['success']=True
finally:
 for i,o in enumerate(owners):
  try:o.close()
  except (AssertionError,BrokenPipeError) as error:result['cleanup_error']=str(error)
  finally:(root/f'owner-{i}.log').write_bytes(o.err)
 probe.save(root/'result.json',result)
print(json.dumps({k:v for k,v in result.items() if k not in ('samples','operations')},indent=2))
