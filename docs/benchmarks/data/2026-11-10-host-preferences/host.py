#!/usr/bin/env python3
"""Exercise the whole host policy on isolated ordinary owners, with live speech."""
import argparse,concurrent.futures,hashlib,json,os,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'2026-11-10-resident-reload'))
import resident

def reload(peer, kind='reload_host_preferences_v1'):
    start=time.perf_counter_ns();state=peer.status();request=dict(incarnation=state['incarnation'],sequence=(state['last_operation'] or {}).get('sequence',0)+1,expected_revision=state['revision'])
    reply=peer.rpc(kind,**request);deadline=time.monotonic()+5
    while reply['outcome'] in ['preparing','pending']:
        assert time.monotonic()<deadline,reply
        time.sleep(.001);state=peer.status();reply=dict(outcome=state['last_operation']['outcome'],state=state)
    end=time.perf_counter_ns()
    return dict(start_ns=start,end_ns=end,ms=(end-start)/1e6,request=request,**reply)

def configuration(n=0, engine="flite"):
    config=resident.configuration(f'punctuation {n}', engine)
    config['speech'].update(max_chunk_words=15+n%30,capital_pitch=dict(default='off' if n%2 else 1.5),defaults=dict(rate=.4+(n%5)*.05,voice_volume=.8))
    return config

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('exe',type=Path);parser.add_argument('output',type=Path);parser.add_argument('--windows-runtime',type=Path);parser.add_argument('--samples',type=int,default=100);parser.add_argument('--engine',default='flite');args=parser.parse_args()
    root=args.output.resolve();root.mkdir(parents=True);exe=args.exe.resolve()
    env={k:os.environ[k] for k in ['PATH','HOME','LANG','WSL_INTEROP'] if k in os.environ}
    env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE=args.engine,OMNIVOX_AUDIO_OUTPUT='null')
    if args.engine=='piper':
        env.update(OMNIVOX_PIPER_HELPER=str(Path('target/lightweight-investigation/piper-companion/omnivox-piper-helper').resolve()),OMNIVOX_PIPER_MODEL='/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx')
    if args.windows_runtime:
        env['ESPEAK_NG_DATA']=(args.windows_runtime.resolve()/'espeak-ng-data.path').read_text().strip();env['WSLENV']='OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA'
    resident.save(root/'config.json',configuration(engine=args.engine));peers=[]
    result=dict(success=False,sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),checks=[],samples=[],operations=[])
    try:
        for i in range(2):
            peer=resident.Peer(exe,env,root,i);peers.append(peer)
            assert 'host_preferences_reload_v1' in peer.capabilities['features'];peer.register();peer.speech()
        initial=[p.status() for p in peers]
        # A host-only edit changes the complete policy identity, not punctuation.
        cfg=configuration(engine=args.engine);cfg['speech']['max_chunk_words']=50;resident.save(root/'config.json',cfg)
        assert reload(peers[0],'reload_punctuation_v1')['outcome']=='rejected'
        row=reload(peers[0]);assert row['outcome']=='applied';result['operations'].append(row)
        assert row['state']['punctuation_sha256']==initial[0]['punctuation_sha256']
        assert row['state']['host_preferences_sha256']!=initial[0]['host_preferences_sha256']
        assert peers[0].rpc('reload_host_preferences_v1',**row['request'])['outcome']=='applied'
        assert peers[0].rpc('reload_punctuation_v1',**row['request'])['outcome']=='stale'
        cfg['speech']['punctuation_profiles']['probe']['overrides']['!']='after host reload';resident.save(root/'config.json',cfg)
        assert reload(peers[0],'reload_punctuation_v1')['outcome']=='applied'
        result['checks'].append('host-only identity, scope-safe retry and subsequent punctuation reload')
        cfg['engine_overrides']['espeak']=dict(enabled=False);resident.save(root/'config.json',cfg)
        assert reload(peers[0])['outcome']=='rejected';peers[0].speech();result['checks'].append('native change rejected without losing speech')
        (root/'config.json').write_text('{broken');assert reload(peers[0])['outcome']=='rejected';peers[0].speech()
        result['checks'].append('malformed configuration retains working speech')
        for n in range(1,args.samples+1):
            resident.save(root/'config.json',configuration(n,args.engine))
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                futures=[pool.submit(reload,p) for p in peers]
                result['samples'].append(peers[0].speech(workload='letter'))
                for lane,future in enumerate(futures):
                    row=future.result();assert row['outcome']=='applied',row;result['operations'].append(dict(lane=lane,**row))
            result['samples'].append(dict(lane=1,**peers[1].speech()))
        assert all(p.status()['incarnation']==s['incarnation'] for p,s in zip(peers,initial))
        assert all(p.status()['startup_host_preferences_sha256']==s['host_preferences_sha256'] for p,s in zip(peers,initial))
        result['checks'].append('both owners retained across complete policy reloads')
        peers[1].close();peers.pop();resident.save(root/'config.json',configuration(args.samples+1,args.engine));assert reload(peers[0])['outcome']=='applied';peers[0].speech()
        result['checks'].append('foreground reload and speech with secondary absent');result['success']=True
    finally:
        for p in peers:p.close()
        resident.save(root/'result.json',result)
    print(json.dumps({k:v for k,v in result.items() if k not in ['samples','operations']},indent=2))
