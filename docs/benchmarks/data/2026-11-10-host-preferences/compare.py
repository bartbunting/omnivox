#!/usr/bin/env python3
"""Four rotated matched blocks for the complete host preference extension."""
import argparse,concurrent.futures,hashlib,json,os,subprocess,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import host
resident=host.resident
parser=argparse.ArgumentParser();parser.add_argument('output',type=Path);parser.add_argument('--exe',type=Path);parser.add_argument('--reload',action='store_true');parser.add_argument('--workload',default='ordered');args=parser.parse_args()
root=args.output.resolve();root.mkdir(parents=True)
if args.exe:
    exe=args.exe.resolve();env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ};env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null')
    resident.save(root/'config.json',host.configuration(engine='espeak'));peers=[];result=dict(success=False,sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),samples=[],operations=[])
    try:
        for i in range(2):
            p=resident.Peer(exe,env,root,i);peers.append(p);p.register();p.speech()
        for n in range(128 if args.reload else 256):
            if args.reload:
                resident.save(root/'config.json',host.configuration(n+1,'espeak'))
                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    futures=[pool.submit(host.reload,p) for p in peers]
                    result['samples'].append(peers[0].speech(workload=args.workload))
                    for lane,f in enumerate(futures):
                        row=f.result();assert row['outcome']=='applied',row;result['operations'].append(dict(lane=lane,**row))
                result['samples'].append(dict(lane=1,**peers[1].speech(workload=args.workload)))
            else:result['samples'].append(peers[n%2].speech(workload=args.workload))
        result['success']=True
    finally:
        for p in peers:p.close()
        resident.save(root/'result.json',result)
else:
    cases=[('release',Path('/home/bart/tmp/ov118-bench/release/omnivox'),False),('punctuation',Path('target/resident-reload/payload-final/omnivox').resolve(),False),('host-idle',Path('target/host-preferences/payload/omnivox').resolve(),False),('host-reload',Path('target/host-preferences/payload/omnivox').resolve(),True)];progress=[]
    for workload in ['ordered','letter']:
        for block in range(4):
            for name,exe,reload in cases[block:]+cases[:block]:
                output=root/f'{workload}-{block}-{name}';cmd=[sys.executable,str(Path(__file__).resolve()),str(output),'--exe',str(exe),'--workload',workload]
                if reload:cmd.append('--reload')
                before=os.getloadavg();start=time.time()
                with (root/f'{output.name}.log').open('w') as log:run=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,timeout=120)
                progress.append(dict(workload=workload,block=block,case=name,exit=run.returncode,start=start,end=time.time(),load_before=before,load_after=os.getloadavg()));resident.save(root/'progress.json',progress)
                assert run.returncode==0,output
                print(workload,block,name,'PASS',flush=True)
    print('PASS: 8192 completed requests')
