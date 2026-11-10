#!/usr/bin/env python3
"""Real engine source timing and reload through independent private owners."""
from common import *
metadata=json.load(open(sys.argv[1]));exe=metadata['executable']
root=Path(sys.argv[2]);root.mkdir(parents=True,exist_ok=False)
engines=['espeak','flite','winrt','eloquence','dectalk','rutts','tgspeechbox','rhvoice']
config={'schema':4,'engine_overrides':{k:{'enabled':False} for k in ['macos','piper','mbrola']}}
results=[]
for repeat in range(int(os.environ.get('REPEATS','2'))):
 for guarded in ([False,True] if repeat%2==0 else [True,False]):
  folder=root/f'{repeat}-{guarded}';folder.mkdir();resident.save(folder/'config.json',config)
  env={k:os.environ[k] for k in ['HOME','PATH','LANG','WSL_INTEROP'] if k in os.environ}
  env.update(OMNIVOX_CONFIG_DIR=str(folder),OMNIVOX_VOICE_ROOT=str(folder/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null',ESPEAK_NG_DATA=metadata['data'],OMNIVOX_STARTUP_TIMING='1')
  env['WSLENV']='OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA:OMNIVOX_STARTUP_TIMING'
  env.update(OMNIVOX_RHVOICE_LIBRARY=metadata['rhvoice_library'],OMNIVOX_RHVOICE_DATA=metadata['rhvoice_data']);env['WSLENV']+=':OMNIVOX_RHVOICE_LIBRARY:OMNIVOX_RHVOICE_DATA'
  peers=[];row=dict(guarded=guarded,repeat=repeat,engines={});results.append(row)
  try:
   for lane in ['main','notify']:
    p=Peer([exe,'--voice-library-owner']+(['--guarded'] if guarded else []),env,folder,lane,'flite');peers.append(p);p.register()
    row[lane+'_ready_ms']=(p.ready-p.started)/1e6
   p,notice=peers
   inventory=p.rpc('inventory');resident.save(folder/'inventory.json',inventory)
   for n,engine in enumerate(engines):
    assert next(e for e in inventory['engines'] if e['id']==engine)['availability']['status']=='available',engine
    p.rpc('set_routing_policy',routing_policy_generation=n+1,preferred_engine_ids=[engine],fallback_engine_ids=[],disabled_engine_ids=[]);p.engine=engine
    text='Б' if engine=='rutts' else 'a';samples=[];reloads=[]
    p.speech('Проверка речи.' if engine=='rutts' else 'Engine qualification.')
    with concurrent.futures.ThreadPoolExecutor(2) as pool:
     for i in range(int(os.environ.get('SAMPLES','40'))):
      candidate=dict(config,speech=resident.configuration(str(n)+'-'+str(i))['speech']);resident.save(folder/'config.json',candidate)
      p.reload_submitted=threading.Event();reload=pool.submit(p.reload);notification=pool.submit(notice.speech,'n','letter')
      assert p.reload_submitted.wait(6)
      samples.append(p.speech(text,'letter'));reloads.append(reload.result());notification.result()
      assert reloads[-1]['outcome']=='applied',reloads[-1]
    row['engines'][engine]=dict(samples=samples,reloads=reloads,median_ms=statistics.median(s['source_ms'] for s in samples))
    print(repeat,guarded,engine,row['engines'][engine]['median_ms'],flush=True)
   row['passed']=True
  finally:
   for p in peers:p.close()
   resident.save(root/'result.json',dict(runtime=metadata,runs=results))
