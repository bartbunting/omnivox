#!/usr/bin/env python3
"""Unavailable preferred helper retains usable fallback and reload."""
import os,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import resident
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]).resolve();root.mkdir(parents=True)
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='flite',OMNIVOX_FLITE_HELPER=str(root/'missing-flite-helper'),OMNIVOX_AUDIO_OUTPUT='null')
config=resident.configuration(engine='flite');resident.save(root/'config.json',config)
start=time.perf_counter_ns();peer=resident.Peer(exe,env,root,0);peer.engine='espeak'
result=dict(success=False)
try:
    speech=peer.speech();result['first_usable_source_ms']=(speech['start_ns']-start)/1e6+speech['source_ms']
    config['speech']['punctuation_profiles']['probe']['overrides']['!']='fallback changed';resident.save(root/'config.json',config)
    result['reload']=peer.reload();assert result['reload']['outcome']=='applied';result['speech']=peer.speech();result['success']=True
finally:peer.close();resident.save(root/'result.json',result)
print('PASS: unavailable preferred Flite falls back to eSpeak; same worker reloads and speaks')
