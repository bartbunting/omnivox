import sys,os,json,time,subprocess,copy,hashlib
from pathlib import Path
sys.dont_write_bytecode=True
repo=Path('/home/bart/src/omnivox-lightweight-reload');sys.path.insert(0,str(repo/'docs/benchmarks/data/2026-11-10-replacement-and-handover/fixtures'))
from common import Peer
meta=json.loads((repo/'docs/benchmarks/data/2026-11-10-helper-queue/listening/runtime.json').read_text());meta['executable']=meta['executable'].replace('ea5d6dfb93dd575d','4e1c1561f51908a6')
root=Path('/mnt/c/Users/bart/AppData/Local/Temp')/('omnivox-early-check-'+str(time.time_ns()));root.mkdir()
def win(p):return subprocess.check_output(['wslpath','-w',str(p)],text=True).strip()
source=(repo/'test-fixtures/framework-helper.cs').read_text().replace('string line;', 'if (options.ContainsKey("--delay")) Thread.Sleep(Int32.Parse(options["--delay"]));\n        string line;')
source=source.replace('Console.InputEncoding = new UTF8Encoding(false);','Console.SetIn(new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false)));').replace('Console.OutputEncoding = new UTF8Encoding(false);','Console.SetOut(new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true });')
(root/'helper.cs').write_text(source)
subprocess.run(['/mnt/c/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe','/nologo','/target:winexe','/reference:System.Web.Extensions.dll','/out:'+win(root/'helper.exe'),win(root/'helper.cs')],check=True,cwd=root)
descriptor=json.loads((repo/'docs/protocol-fixtures/control-inventory-response.json').read_text())['engines'][0];descriptor.update(id='rutts',display_name='Delayed fixture',default_voice_id='voice');descriptor['voices'][0]['id']=dict(engine_id='rutts',voice_id='voice');descriptor['capabilities']['cancellation']='synthesis_and_playback';(root/'descriptor.json').write_text(json.dumps(descriptor))
rows=[]
def run(name,early,delay=4000,retire_pending=False,fail_optional=False,fail_selected=False):
 case=root/name;case.mkdir();cfg=dict(schema=4,engine_overrides={k:dict(enabled=False) for k in ['winrt','macos','piper','mbrola','rhvoice','eloquence','dectalk','tgspeechbox']})
 cfg['engine_overrides']['rutts']=dict(program=win(root/'helper.exe'),arguments=['--descriptor',win(root/'descriptor.json'),'--record',win(case/'calls.jsonl'),'--delay',str(delay)],timeouts=dict(startup_ms=1000 if fail_optional else 10000))
 if fail_selected:cfg['engine_overrides']['flite']=dict(program=win(root/'missing.exe'))
 (case/'config.json').write_text(json.dumps(cfg))
 env={k:os.environ[k] for k in ['HOME','PATH','LANG','WSL_INTEROP'] if k in os.environ};env.update(OMNIVOX_CONFIG_DIR=win(case),OMNIVOX_VOICE_ROOT=win(case/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null',ESPEAK_NG_DATA=meta['data'],OMNIVOX_EXPERIMENTAL_EARLY_SPEECH='1' if early else '0');env['WSLENV']=':'.join(k for k in env if k.startswith('OMNIVOX_') or k=='ESPEAK_NG_DATA')
 p=Peer([meta['executable'],'--voice-library-owner','--guarded'],env,case,'main','espeak' if fail_selected else 'flite')
 row=dict(name=name,early=early,ready_ms=(p.ready-p.started)/1e6); notification=None
 try:
  first=p.speech();row['first_source_ms']=(first['start_ns']-p.started)/1e6+first['source_ms']; inv=p.rpc('inventory');optional=next(e for e in inv['engines'] if e['id']=='rutts');row['initial_optional']=optional
  if early:
   assert optional['availability']['status']=='unavailable' and not optional['voices'],optional
   assert row['first_source_ms']<delay,row
  if retire_pending:
   notification=Peer([meta['executable'],'--voice-library-owner','--guarded'],env,case,'notify','flite');notification.speech();p.send(b's\n');p.speech();row['retired_while_pending']=True
  else:
   deadline=time.monotonic()+12;row['during']=[]
   while optional['availability']['status']!='available' and time.monotonic()<deadline:
    row['during'].append(p.speech());p.send(b's\n');time.sleep(.05)
    optional=next(e for e in p.rpc('inventory')['engines'] if e['id']=='rutts')
    if fail_optional and 'progress' not in optional['availability'].get('reason','').lower():break
   row['final_optional']=optional
   if fail_optional:
    assert optional['availability']['status']=='unavailable';p.speech()
   else:
    assert optional['availability']['status']=='available',optional
    p.rpc('set_routing_policy',routing_policy_generation=1,preferred_engine_ids=['rutts'],fallback_engine_ids=[],disabled_engine_ids=[]);p.engine='rutts';p.speech('Later discovered engine.')
  row['passed']=True
 finally:
  started=time.perf_counter();row['exit']=p.close();row['retire_ms']=(time.perf_counter()-started)*1000
  if notification is not None:
   notification.speech();row['notification_survived_retirement']=True;assert notification.close()==0
  rows.append(row);(root/'result.json').write_text(json.dumps(dict(runtime=meta,runs=rows),indent=2));print(name,row['first_source_ms'],row.get('passed'),flush=True)
for i,early in enumerate([False,True,True,False]):run(str(i),early)
run('retire-pending',True,retire_pending=True)
run('optional-fails',True,fail_optional=True)
run('preferred-fails',True,fail_selected=True)
print('ROOT',root,flush=True)
