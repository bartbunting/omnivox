#!/usr/bin/env python3
"""Wall time from capture invocation through two independent installation replies."""
import os,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True);exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex())
probe.save(root/'config.json',probe.configuration());owners=[];rows=[]
try:
 for _ in range(2):owners.append(probe.Owner(exe,env))
 for i in range(100):
  probe.save(root/'config.json',probe.configuration(str(i)))
  start=time.perf_counter_ns();candidate,prep=probe.prepare(exe,env)
  first=owners[0].request(candidate);foreground=(time.perf_counter_ns()-start)/1e6
  second=owners[1].request(candidate);total=(time.perf_counter_ns()-start)/1e6
  assert first['outcome']==second['outcome']=='applied'
  rows.append(dict(prepare_ms=prep,foreground_ms=foreground,total_ms=total,first=first,second=second))
  for o in owners:o.speech()
finally:
 for i,o in enumerate(owners):o.close();(root/f'owner-{i}.log').write_bytes(o.err)
 probe.save(root/'result.json',dict(success=len(rows)==100,operations=rows))
print('PASS 100 full capture-and-two-worker reload operations')
