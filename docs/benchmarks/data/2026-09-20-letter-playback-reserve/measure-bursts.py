#!/usr/bin/env python3
"""Muted device acceptance for rapid l replacement, stop, and recovery."""
import argparse, importlib.util, json, os, re, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('output',type=Path); p.add_argument('--program',required=True)
a=p.parse_args(); a.output.mkdir(parents=True,exist_ok=False)
os.environ.update(OMNIVOX_PROGRAM=a.program, OMNIVOX_DECTALK_DLL='C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll', ESPEAK_NG_DATA='C:/Users/bart/AppData/Local/Emacsvox/Omnivox/espeak-data/9a2e176369cfe0b982737abf6c26037f21d2b9af2a2fabcc08062864d0157718', RUST_LOG='info,omnivox_audio::output=debug',EMACSVOX_OMNIVOX_DIAGNOSTIC='1',OMNIVOX_LOG_DIRECTORY=str(a.output/'logs'))
spec=importlib.util.spec_from_file_location('b','/home/bart/src/omnivox/tools/benchmark_server.py');b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class Session(b.ServerSession):
 def _read_stderr(self):
  for line in self.process.stderr:self.stderr_lines.append(line)
s=Session(['/home/bart/src/emacsvox/servers/omnivox','--audio-output','device'],'dectalk',20)
ranges=[];epoch=0
try:
 cap,_=s.negotiate(10000);b.configure_preferred_engine(s,cap,'dectalk',10001)
 for line in ['tts_set_voice paul','tts_set_speech_rate 85','tts_set_character_scale 1.1','tts_set_voice_volume 0']:s.send_line(line)
 for gap in [100,50,30,20,10]:
  start=epoch+1
  for i in range(60):
   s.send_line('l {'+'abwA'[i%4]+'}');epoch+=1;time.sleep(gap/1000)
  time.sleep(.6);ranges.append({'gap_ms':gap,'first':start,'last':epoch})
 # Hard stops followed by new navigation must leave the helper usable.
 for i in range(10):
  s.send_line('l {w}');epoch+=1;time.sleep(.005);s.send_line('s');epoch+=1;time.sleep(.08)
  s.send_line('l {a}');epoch+=1;time.sleep(.3)
 time.sleep(.5)
finally:s.close();(a.output/'stderr.log').write_text(''.join(s.stderr_lines))
starts={};routes={}
for line in s.stderr_lines:
 if 'request_kind="letter"' not in line:continue
 m=re.search(r'\bstop_epoch=(\d+)',line)
 if not m:continue
 e=int(m.group(1))
 if 'lifecycle_stage="mixer_source_started"' in line:
  starts[e]=int(re.search(r'admission_to_mixer_source_us=Some\((\d+)\)',line).group(1))/1000
 if 'lifecycle_stage="synthesis_started"' in line:
  routes[e]={k:re.search(r'\b'+k+r'="([^"]+)"',line).group(1) for k in ['engine_id','voice_id']}
assert all(r=={'engine_id':'dectalk','voice_id':'paul'} for r in routes.values()),routes
summary=[]
for run in ranges:
 values=[value for e,value in starts.items() if run['first']<=e<=run['last']]
 assert run['last'] in starts,('last navigation did not start',run)
 assert starts[run['last']]<1000,('recovery exceeded one second',run)
 summary.append({**run,'started':len(values),'source_p50_ms':b.nearest_rank(values,.5),'source_p95_ms':b.nearest_rank(values,.95),'source_max_ms':max(values)})
for e in range(ranges[-1]['last']+3,epoch+1,3):assert e in starts,('hard stop prevented recovery',e)
(a.output/'results.json').write_text(json.dumps({'summary':summary,'source_ms_by_epoch':starts,'routes':routes,'hard_stop_recoveries':10},indent=2)+'\n')
print(json.dumps(summary,indent=2))
