#!/usr/bin/env python3
import json,os,signal,sys,time
from pathlib import Path
sys.dont_write_bytecode=True
import probe
root=Path(sys.argv[2]).resolve();root.mkdir(parents=True);exe=Path(sys.argv[1]).resolve()
env={k:os.environ[k] for k in ['PATH','HOME','LANG'] if k in os.environ}
env.update(OMNIVOX_CONFIG_DIR=str(root),OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_ENGINE='espeak',OMNIVOX_AUDIO_OUTPUT='null',OMNIVOX_EXPERIMENTAL_RELOAD_TOKEN=os.urandom(32).hex());probe.save(root/'config.json',probe.configuration())
rows=[];o=peer=None
try:
 peer=probe.Owner(exe,env)
 for iteration in range(5):
  o=probe.Owner(exe,env);o.speech()
  o.send(b'OMNIVOX-LOCAL '+probe.record(dict(request_id=99,command='describe'))+b'\n');owner=json.loads(o.line().split(b' ',1)[1])
  worker=int(Path(f'/proc/{o.p.pid}/task/{o.p.pid}/children').read_text().split()[0])
  # Live fixture child only; retain a pidfd while verifying membership again.
  handle=os.pidfd_open(worker)
  assert str(worker) in Path(f'/proc/{o.p.pid}/task/{o.p.pid}/children').read_text().split()
  begin=time.perf_counter_ns();signal.pidfd_send_signal(handle,signal.SIGKILL);os.close(handle)
  peer.speech();o.close();o=None
  recovered=probe.Owner(exe,dict(env,OMNIVOX_OWNED_STARTUP=owner['startup'],OMNIVOX_OWNED_STARTUP_SHA256=owner['startup_sha256']))
  observed=recovered.speech();recovered.close()
  rows.append(dict(iteration=iteration,crash_to_completed_probe_ms=(time.perf_counter_ns()-begin)/1e6,source_ms=observed['source_ms']))
finally:
 if o:o.close()
 if peer:peer.close()
probe.save(root/'result.json',dict(success=True,checks=['pidfd-bound worker crash','peer speech survives','owned cleanup before replacement','retained recovery'],cycles=rows));print(rows)
