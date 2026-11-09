#!/usr/bin/env python3
"""Matched owned-stdio source-event measurements, isolated null output only."""
import argparse, base64, concurrent.futures, hashlib, json, os, subprocess, threading, time
from pathlib import Path


def encoded(value): return base64.b64encode(json.dumps(value,separators=(',',':')).encode())
def save(path,value): path.write_text(json.dumps(value,indent=2)+'\n')
def configuration(word='before',engine='espeak'):
    disabled=('winrt','macos','piper','rhvoice','flite','rutts','tgspeechbox','eloquence','dectalk','mbrola')
    return dict(schema=4,engine_overrides={k:dict(enabled=False) for k in disabled if k!=engine},speech=dict(punctuation_profiles=dict(probe=dict(base='all',overrides={'!':word}))))

class Peer:
    def __init__(self,exe,env,root,index,owned=True):
        self.p=subprocess.Popen([str(exe)]+(['--voice-library-owner'] if owned else []),env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=open(root/f'peer-{index}.stderr','wb'),bufsize=0)
        self.rows=[]; self.condition=threading.Condition();self.control=0;self.dispatch=0;self.engine=env['OMNIVOX_ENGINE'];self.eof=False
        self.log=open(root/f'peer-{index}.jsonl','w');self.write_lock=threading.Lock()
        def read():
            for line in self.p.stdout:
                now=time.perf_counter_ns()
                if line.startswith(b'__OMNIVOX_CONTROL__ '):
                    value=json.loads(base64.b64decode(line.split()[1]));kind='control'
                elif line.startswith(b'__EMACSVOX_MARKER__ '):
                    value=json.loads(base64.b64decode(line.split()[1]));kind='marker'
                elif line.startswith(b'__EMACSVOX_TRACKED__ '):
                    fields=line.split();value=dict(dispatch_id=int(fields[1]),outcome=fields[2].decode());kind='terminal'
                else: value=dict(line=line.decode(errors='replace'));kind='other'
                row=dict(ns=now,kind=kind,value=value)
                self.log.write(json.dumps(row)+'\n')
                with self.condition:self.rows.append(row);self.condition.notify_all()
            with self.condition:self.eof=True;self.condition.notify_all()
        self.reader=threading.Thread(target=read,daemon=True);self.reader.start()
        self.capabilities=self.rpc('capabilities')
    def send(self,data):
        with self.write_lock:self.p.stdin.write(data);self.p.stdin.flush()
    def wait(self,predicate,timeout=15):
        deadline=time.monotonic()+timeout
        with self.condition:
            while True:
                for i,row in enumerate(self.rows):
                    if predicate(row):return self.rows.pop(i)
                assert not self.eof, 'unexpected EOF'
                remaining=deadline-time.monotonic();assert remaining>0,'response timeout'
                self.condition.wait(remaining)
    def rpc(self,kind,**fields):
        with self.write_lock:
            self.control+=1;identity=self.control
            data=encoded(dict(protocol_version=1,request_id=identity,type=kind,**fields))
            self.p.stdin.write(b'omnivox_control '+data+b'\n');self.p.stdin.flush()
        return self.wait(lambda r:r['kind']=='control' and r['value'].get('request_id')==identity)['value']
    def status(self):return self.rpc('host_preferences_status_v1')
    def reload(self,state=None):
        start=time.perf_counter_ns();state=state or self.status();seq=(state.get('last_operation') or {}).get('sequence',0)+1
        request=dict(incarnation=state['incarnation'],sequence=seq,expected_revision=state['revision'])
        reply=self.rpc('reload_punctuation_v1',**request)
        response_ns=time.perf_counter_ns();deadline=time.monotonic()+5
        while reply['outcome'] in ('preparing','pending'):
            assert time.monotonic()<deadline,reply
            time.sleep(.001);state=self.status();reply=dict(outcome=state['last_operation']['outcome'],state=state)
        end=time.perf_counter_ns()
        return dict(start_ns=start,end_ns=end,ms=(end-start)/1e6,response_ms=(response_ns-start)/1e6,request=request,**reply)
    def speech(self,text='A',workload='ordered'):
        self.dispatch+=1;identity=self.dispatch
        value=dict(protocol_version=3,generation=identity,dispatch_id=identity,delivery_policy=workload,spans=[dict(id=1,text=text)],actions=[])
        if workload=='replaceable':value['replacement_key']='probe'
        command=b'emacsvox_timeline '
        if workload=='letter':
            command=b'emacsvox_letter ';value.update(protocol_version=4,registry_generation=1,delivery_policy='ordered',spans=[dict(mode='layered',span=dict(id=1,text=text,logical_voice_id='probe',context={},placement=dict(pan=None)))])
        start=time.perf_counter_ns();self.send(command+encoded(value)+b'\n')
        marker=self.wait(lambda r:r['kind']=='marker' and r['value']['dispatch_id']==identity and r['value']['type']=='utterance_started')
        assert marker['value']['engine_id']==self.engine,marker
        terminal=self.wait(lambda r:r['kind']=='terminal' and r['value']['dispatch_id']==identity)
        assert terminal['value']['outcome']=='completed',terminal
        with self.condition:self.rows[:]=[r for r in self.rows if not(r['kind']=='marker' and r['value']['dispatch_id']==identity)]
        return dict(id=identity,start_ns=start,end_ns=terminal['ns'],source_ms=(marker['ns']-start)/1e6,terminal_ms=(terminal['ns']-start)/1e6)
    def register(self):
        return self.rpc('register_logical_voices_v2',registry_generation=1,definitions=[dict(mode='layered',definition=dict(id='probe',language=None,shared=dict(acss={k:None for k in ['rate','average_pitch','pitch_range','stress','richness','volume']},rate_offset=None,effects={k:None for k in ['gain','low_pass','high_pass','pan','reverb','echo','chorus']}),choices=[]))],fallback_policy=dict(preferred_engines=[self.engine],allow_same_language_on_requested_engine=True,global_default=None,fallback_engines=[]))
    def close(self):
        self.p.stdin.close();self.p.wait(timeout=10);self.reader.join(timeout=2);self.log.close();self.p.stdout.close();assert self.p.returncode==0,self.p.returncode

def run(args):
    root=args.output.resolve();root.mkdir(parents=True);exe=args.exe.resolve()
    env={k:os.environ[k] for k in ('PATH','HOME','LANG','WSL_INTEROP') if k in os.environ}
    env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE=args.engine,OMNIVOX_AUDIO_OUTPUT='null')
    if args.engine=='piper':env.update(OMNIVOX_PIPER_HELPER=str(Path('target/lightweight-investigation/piper-companion/omnivox-piper-helper').resolve()),OMNIVOX_PIPER_MODEL='/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx')
    if args.windows_runtime:
        runtime=args.windows_runtime.resolve();env['ESPEAK_NG_DATA']=(runtime/'espeak-ng-data.path').read_text().strip()
        env['WSLENV']='OMNIVOX_CONFIG_DIR/p:OMNIVOX_VOICE_ROOT/p:OMNIVOX_ENGINE:OMNIVOX_AUDIO_OUTPUT:ESPEAK_NG_DATA'
    cfg=configuration(engine=args.engine);save(root/'config.json',cfg)
    result=dict(executable=str(exe),sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),engine=args.engine,workload=args.workload,reload=args.reload,samples=[],operations=[],checks=[],success=False);peers=[]
    try:
        start=time.perf_counter_ns()
        for i in range(2):
            peer=Peer(exe,env,root,i);peers.append(peer)
            peer.register();peer.speech()
        result['two_ready_ms']=(time.perf_counter_ns()-start)/1e6
        if args.reload or args.faults:
            for peer in peers:assert 'host_preferences_v1' in peer.capabilities['features'],peer.capabilities
            result['initial_status']=[p.status() for p in peers]
        if args.faults:
            p=peers[0];original=p.status();catalogue=p.rpc('get_punctuation_profiles_v1')['profiles']
            (root/'config.json').write_text('{broken');r=p.reload();assert r['outcome']=='rejected',r;p.speech();result['checks'].append('malformed configuration preserves speech')
            mixed=configuration('changed',args.engine);mixed['speech']['max_chunk_words']=19;save(root/'config.json',mixed);r=p.reload();assert r['outcome']=='rejected',r;p.speech();result['checks'].append('mixed scope rejects coherently')
            save(root/'config.json',configuration('changed',args.engine));r=p.reload();assert r['outcome']=='applied',r;result['operations'].append(r)
            assert p.rpc('get_punctuation_profiles_v1')['profiles']!=catalogue
            assert p.rpc('reload_punctuation_v1',**r['request'])['outcome']=='applied'
            save(root/'config.json',cfg);r2=p.reload();assert r2['outcome']=='applied';assert p.rpc('reload_punctuation_v1',**r['request'])['outcome']=='stale';result['checks'].append('duplicate and ABA replay safety');assert p.rpc('get_punctuation_profiles_v1')['profiles']==catalogue
            p.send(b'q {old queued text}\n');save(root/'config.json',configuration('queued',args.engine));state=p.status();seq=state['last_operation']['sequence']+1
            p.rpc('reload_punctuation_v1',incarnation=state['incarnation'],sequence=seq,expected_revision=state['revision']);time.sleep(.03)
            assert p.status()['last_operation']['outcome']=='pending';p.send(b's\n');p.speech();assert p.status()['last_operation']['outcome']=='applied';result['checks'].append('pending legacy batch keeps Stop responsive')
            p.send(b'tts_set_punctuation_profile probe\n');without=configuration('other',args.engine);without['speech']['punctuation_profiles']={};save(root/'config.json',without);r=p.reload();assert r['outcome']=='rejected',r;p.speech();result['checks'].append('selected profile retained')
            # A secondary legacy batch delays only its policy, not foreground installation.
            peers[1].send(b'q {secondary held}\n');save(root/'config.json',configuration('independent',args.engine))
            secondary=peers[1].status();secondary_seq=(secondary.get('last_operation') or {}).get('sequence',0)+1
            peers[1].rpc('reload_punctuation_v1',incarnation=secondary['incarnation'],sequence=secondary_seq,expected_revision=secondary['revision'])
            time.sleep(.03);assert peers[1].status()['last_operation']['outcome']=='pending'
            r=p.reload();assert r['outcome']=='applied';p.speech();assert peers[1].status()['last_operation']['outcome']=='pending'
            peers[1].send(b's\n');peers[1].speech();assert peers[1].status()['last_operation']['outcome']=='applied'
            result['checks'].append('pending secondary reload does not gate foreground')
        for i in range(args.samples):
            if args.reload:
                save(root/'config.json',configuration(f'word{i}',args.engine))
                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    futures=[pool.submit(p.reload) for p in peers]
                    result['samples'].append(peers[0].speech(workload=args.workload))
                    for lane,future in enumerate(futures):
                        row=future.result();assert row['outcome'] in ('applied','unchanged'),row;result['operations'].append(dict(lane=lane,**row))
                result['samples'].append(dict(lane=1,**peers[1].speech(workload=args.workload)))
            else:result['samples'].append(peers[i%2].speech(workload=args.workload))
        if args.faults:
            peers[1].close();peers.pop();save(root/'config.json',configuration('alone',args.engine));assert peers[0].reload()['outcome']=='applied';peers[0].speech();result['checks'].append('absent notification keeps foreground reload and speech')
            direct=Peer(exe,env,root,'unowned',owned=False)
            try:assert 'host_preferences_v1' not in direct.capabilities['features'];assert direct.rpc('host_preferences_status_v1')['type']=='error'
            finally:direct.close()
            result['checks'].append('unowned connection denied local reload')
        result['success']=True
    finally:
        for peer in peers:peer.close()
        save(root/'result.json',result)
    print(json.dumps({k:v for k,v in result.items() if k not in ('samples','operations','initial_status')},indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('exe',type=Path);parser.add_argument('output',type=Path);parser.add_argument('--samples',type=int,default=100);parser.add_argument('--reload',action='store_true');parser.add_argument('--faults',action='store_true');parser.add_argument('--engine',default='espeak');parser.add_argument('--workload',choices=['ordered','letter','replaceable'],default='ordered');parser.add_argument('--windows-runtime',type=Path);run(parser.parse_args())
