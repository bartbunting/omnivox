#!/usr/bin/env python3
"""Maximum resolved profiles, read-only reload and native identity retention."""
import json,os,subprocess,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_AUDIO_OUTPUT='null')
config=resident.configuration(engine='flite');resident.save(root/'config.json',config)
review=subprocess.run([str(exe),'--voice-library-service'],env=env,input=b'{"request_id":1,"command":"punctuation-review"}\n',capture_output=True,timeout=10)
assert review.returncode==0,review.stderr
value=json.loads(review.stdout.split(b' ',1)[1]);table=value['review']['defaults']['all'];character=0x4e00
while len(table)<512:table[chr(character)]='name';character+=1
config['speech']['punctuation']={'all':{c:'z'*64 for c in table}}
config['speech']['punctuation_profiles']={f'p{i}':dict(base='all',overrides={}) for i in range(32)}
resident.save(root/'config.json',config);assert (root/'config.json').stat().st_size<128*1024
peer=resident.Peer(exe,env,root,0);rows=[];catalogues=[];original_mode=root.stat().st_mode
try:
    initial=peer.status();previous=peer.rpc('get_punctuation_profiles_v1')['profiles'];assert len(previous)==32
    for i in range(12):
        config['speech']['punctuation']['all']['!']=str(i)*32;resident.save(root/'config.json',config)
        # Reload itself must require no directory writes.
        root.chmod(0o500)
        try:operation=peer.reload()
        finally:root.chmod(original_mode)
        assert operation['outcome']=='applied',operation
        current=peer.rpc('get_punctuation_profiles_v1')['profiles'];assert len(current)==32 and current!=previous
        previous=current;catalogues.append(current);rows.append(dict(operation=operation,speech=peer.speech()))
    assert initial['incarnation']==peer.status()['incarnation']
finally:
    root.chmod(original_mode);peer.close();resident.save(root/'result.json',dict(success=len(rows)==12,rows=rows,catalogues=catalogues,configuration_bytes=(root/'config.json').stat().st_size))
print('PASS: 12 reloads of 32 fully resolved 512-entry profiles; read-only directory; same Flite worker')
