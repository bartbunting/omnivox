#!/usr/bin/env python3
"""Owned isolated native probes. Null source observations, not acoustic timing."""
import argparse, base64, concurrent.futures, copy, hashlib, json, os, select, socket, struct, subprocess, time, traceback
from pathlib import Path


def record(v): return json.dumps(v,separators=(',',':')).encode()
def save(p,v): p.write_text(json.dumps(v,indent=2)+'\n')

class Owner:
    def __init__(self,exe,env):
        self.p=subprocess.Popen([str(exe),'--voice-library-owner'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,bufsize=0)
        self.expected_engine=env['OMNIVOX_ENGINE']; self.tail=b''; self.err=bytearray(); self.endpoint=None; self.token=env.get('OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN'); self.seq=0
        self.send(b'omnivox_control '+base64.b64encode(record(dict(protocol_version=1,request_id=1,type='capabilities')))+b'\n')
        while True:
            line=self.line()
            if line.startswith(b'__OMNIVOX_LIGHTWEIGHT__ '): self.endpoint=json.loads(line.split(b' ',1)[1])
            else:
                assert line.startswith(b'__OMNIVOX_CONTROL__ '),line[:200]
                break
    def send(self,b): self.p.stdin.write(b); self.p.stdin.flush()
    def line(self,timeout=15):
        end=time.monotonic()+timeout
        while b'\n' not in self.tail:
            assert time.monotonic()<end,'reply timeout'
            for stream in select.select([self.p.stdout,self.p.stderr],[],[],.05)[0]:
                b=os.read(stream.fileno(),65536)
                if stream==self.p.stdout:
                    assert b,'unexpected EOF'
                    self.tail+=b
                else: self.err.extend(b)
        line,self.tail=self.tail.split(b'\n',1); return line
    def request(self,candidate=None,expected=None,**override):
        state=self.endpoint['state']
        value=dict(token=self.token,incarnation=state['incarnation'],expected=expected or state['policy'],candidate=candidate)
        value.update(override)
        b=record(value); address=self.endpoint['address'].rsplit(':',1)
        start=time.perf_counter_ns()
        with socket.create_connection((address[0],int(address[1])),timeout=5) as s:
            s.sendall(struct.pack('>I',len(b))+b)
            answer=b''
            while b'\n' not in answer:
                chunk=s.recv(65536); assert chunk,'management EOF'; answer+=chunk
        result=json.loads(answer)
        result['start_ns']=start;result['end_ns']=time.perf_counter_ns()
        result['ms']=(result['end_ns']-start)/1e6
        if 'state' in result: self.endpoint['state']=result['state']
        return result
    def speech(self,text='A',kind='ordered'):
        self.seq+=1; identity=self.seq
        timeline=dict(protocol_version=3,generation=identity,dispatch_id=identity,delivery_policy=('replaceable' if kind=='letter' else kind),spans=[dict(id=1,text=text)],actions=[])
        if kind=='replaceable': timeline['replacement_key']='probe'
        if kind=='letter':
            timeline.update(protocol_version=4,registry_generation=1,delivery_policy='ordered',spans=[dict(mode='layered',span=dict(id=1,text=text,logical_voice_id='probe',context={},placement=dict(pan=None)))])
        start=time.perf_counter_ns(); self.send((b'emacsvox_letter ' if kind=='letter' else b'emacsvox_timeline ')+base64.b64encode(record(timeline))+b'\n'); source=None
        while True:
            line=self.line(); now=time.perf_counter_ns()
            if line.startswith(b'__EMACSVOX_MARKER__ '):
                event=json.loads(base64.b64decode(line.split(b' ',1)[1])); assert event['dispatch_id']==identity
                if event['type']=='utterance_started':
                    assert event['engine_id']==self.expected_engine,event
                    source=now
            elif line.startswith(b'__EMACSVOX_TRACKED__ '):
                fields=line.split(); assert int(fields[1])==identity and fields[2]==b'completed',line
                assert source is not None
                return dict(id=identity,start_ns=start,source_ms=(source-start)/1e6,terminal_ms=(now-start)/1e6)
            else: raise AssertionError(line[:200])
    def close(self):
        self.p.stdin.close(); self.p.wait(timeout=15)
        self.err.extend(self.p.stderr.read()); self.p.stdout.close(); self.p.stderr.close()
        assert self.p.returncode==0,self.p.returncode

def configuration(word='before'):
    disabled=('winrt','macos','piper','rhvoice','flite','rutts','tgspeechbox','eloquence','dectalk','mbrola')
    return dict(schema=4,engine_overrides={k:dict(enabled=False) for k in disabled},speech=dict(punctuation_profiles=dict(probe=dict(base='all',overrides={'!':word}))))

def prepare(exe,env):
    start=time.perf_counter_ns()
    p=subprocess.run([str(exe),'--experimental-reload-candidate'],env=env,capture_output=True,timeout=15)
    assert p.returncode==0,p.stderr.decode()
    return json.loads(p.stdout),(time.perf_counter_ns()-start)/1e6

def run(exe,root,samples,experimental=False,faults=False,reloads=0,workload="ordered",engine="espeak"):
    root=root.resolve()
    root.mkdir(parents=True)
    env={k:os.environ[k] for k in ('PATH','HOME','LANG') if k in os.environ}
    env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE=engine,OMNIVOX_AUDIO_OUTPUT='null')
    if engine=='piper':
        env.update(OMNIVOX_PIPER_HELPER=str(Path('target/lightweight-investigation/piper-companion/omnivox-piper-helper').resolve()),OMNIVOX_PIPER_MODEL='/home/bart/src/omnivox/target/piper-test-model/en_US-lessac-medium.onnx')
    if experimental: env['OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN']=os.urandom(32).hex()
    cfg=configuration(); cfg['engine_overrides'].pop(engine,None); save(root/'config.json',cfg)
    owners=[]; result=dict(executable=str(exe),sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),workload=workload,engine=engine,samples=[],operations=[],checks=[],success=False)
    try:
        for _ in range(2): owners.append(Owner(exe,env))
        for o in owners:
            o.speech()
            if workload=='letter':
                request=dict(protocol_version=1,request_id=2,type='register_logical_voices_v2',registry_generation=1,definitions=[dict(mode='layered',definition=dict(id='probe',language=None,shared=dict(acss={k:None for k in ['rate','average_pitch','pitch_range','stress','richness','volume']},rate_offset=None,effects={k:None for k in ['gain','low_pass','high_pass','pan','reverb','echo','chorus']}),choices=[]))],fallback_policy=dict(preferred_engines=[engine],allow_same_language_on_requested_engine=True,global_default=None,fallback_engines=[]))
                o.send(b'omnivox_control '+base64.b64encode(record(request))+b'\n')
                response=json.loads(base64.b64decode(o.line().split(b' ',1)[1])); assert response.get('registry_generation')==1,response
        if experimental:
            updated=configuration('after'); updated['engine_overrides'].pop(engine,None); save(root/'config.json',updated)
            a,prep=prepare(exe,env); result['prepare_ms']=prep
            before=owners[0].endpoint['state']['policy']
            for o in owners:
                r=o.request(a); assert r['outcome']=='applied',r; result['operations'].append(r)
            if faults:
                o=owners[0]
                assert o.request(a,expected=before)['outcome']=='unchanged'
                assert o.request(a,token='0'*64)['outcome']=='rejected'
                assert o.request(a,incarnation='wrong')['outcome']=='rejected'
                bad=copy.deepcopy(a);bad['speech']['max_chunk_words']=19
                assert o.request(bad)['outcome']=='rejected'
                bad=copy.deepcopy(a);bad['punctuation']['profiles']['probe']['table']['!']='third'
                assert o.request(bad,expected=before)['outcome']=='stale'
                # A stalled secondary management connection cannot gate main.
                addr=owners[1].endpoint['address'].rsplit(':',1)
                with socket.create_connection((addr[0],int(addr[1]))) as stalled:
                    stalled.sendall(b'\x00')
                    r=o.request(bad); assert r['outcome']=='applied'; result['operations'].append(r)
                    o.speech()
                owners[1].close(); owners.pop()
                assert o.request(a)['outcome']=='applied'
                o.speech()
                result['checks'] += ['duplicate','bad authentication','stale incarnation','mixed scope','stale retry','stalled secondary','absent secondary']
                # Held legacy text delays preference installation, but not Stop.
                o.send(b'q {queued old text}\n')
                r=o.request(bad); assert r['outcome']=='pending',r
                o.send(b's\n')
                o.speech()
                assert o.request()['state']['policy'] != before
                result['checks'].append('Stop releases queued legacy boundary')
                save(root/'config.json',{'schema':999})
                p=subprocess.run([str(exe),'--experimental-reload-candidate'],env=env,capture_output=True,timeout=15)
                assert p.returncode!=0; o.speech()
                result['checks'].append('malformed file preserves speech')
                updated=configuration('after'); updated['engine_overrides'].pop(engine,None); save(root/'config.json',updated)
            b=copy.deepcopy(a);b['punctuation']['profiles']['probe']['table']['!']='alternate'
        def change():
            for i in range(reloads):
                for lane,o in enumerate(owners):
                    r=o.request(b if i%2==0 else a); assert r['outcome'] in ('applied','unchanged'),r
                    result['operations'].append(dict(lane=lane,**r))
                time.sleep(.003)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            future=pool.submit(change)
            for i in range(samples):
                for lane,o in enumerate(owners): result['samples'].append(dict(lane=lane,**o.speech(text=('A' if workload!='long' else 'A longer utterance. '*80),kind=('ordered' if workload=='long' else workload))))
            future.result()
        result['success']=True
    except BaseException:
        result['failure']=traceback.format_exc(); raise
    finally:
        for i,o in enumerate(owners):
            o.close(); (root/f'owner-{i}-stderr.log').write_bytes(o.err)
        save(root/'result.json',result)
    return result

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('executable',type=Path);p.add_argument('output',type=Path);p.add_argument('--experimental',action='store_true');p.add_argument('--faults',action='store_true');p.add_argument('--samples',type=int,default=40);p.add_argument('--reloads',type=int,default=0);p.add_argument('--workload',choices=['ordered','replaceable','letter','long'],default='ordered');p.add_argument('--engine',default='espeak')
    a=p.parse_args(); r=run(a.executable.resolve(),a.output.resolve(),a.samples,a.experimental,a.faults,a.reloads,a.workload,a.engine)
    print(json.dumps({k:v for k,v in r.items() if k not in ('samples','operations')},indent=2))
