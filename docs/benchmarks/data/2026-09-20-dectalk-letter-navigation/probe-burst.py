import argparse,importlib.util,json,os,re,time,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('output',type=Path);p.add_argument('--program',required=True);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
os.environ.update(OMNIVOX_PROGRAM=a.program,OMNIVOX_DECTALK_DLL='C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll',ESPEAK_NG_DATA='C:/Users/bart/AppData/Local/Emacsvox/Omnivox/espeak-data/9a2e176369cfe0b982737abf6c26037f21d2b9af2a2fabcc08062864d0157718',RUST_LOG='info',EMACSVOX_OMNIVOX_DIAGNOSTIC='1',OMNIVOX_LOG_DIRECTORY=str(a.output/'logs'))
spec=importlib.util.spec_from_file_location('b','/home/bart/src/omnivox/tools/benchmark_server.py');b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class Session(b.ServerSession):
 def _read_stderr(self):
  for line in self.process.stderr:self.stderr_lines.append(line)
s=Session(['/home/bart/src/emacsvox/servers/omnivox','--audio-output','null'],'dectalk',20);ranges=[];epoch=0
try:
 cap,_=s.negotiate(10000);b.configure_preferred_engine(s,cap,'dectalk',10001)
 s.send_line('tts_set_voice paul');s.send_line('tts_set_speech_rate 85');s.send_line('tts_set_character_scale 1.1')
 for gap in [100,50,30,20]:
  first=epoch+1
  for i in range(63):
   s.send_line('l {'+'abw'[i%3]+'}');epoch+=1;time.sleep(gap/1000)
  time.sleep(0.5);ranges.append({'gap_ms':gap,'first_epoch':first,'last_epoch':epoch})
finally:s.close();(a.output/'stderr.log').write_text(''.join(s.stderr_lines))
rows=[]
for line in s.stderr_lines:
 if 'lifecycle_stage="worker_finished"' not in line:continue
 def val(key):
  m=re.search(key+r'=(?:Some\()?([0-9]+)',line);return int(m.group(1)) if m else None
 ep=val('stop_epoch');trial=next((r for r in ranges if r['first_epoch']+3<=ep<=r['last_epoch']),None)
 if trial is None:continue
 rows.append({'gap_ms':trial['gap_ms'],'epoch':ep,**{key.replace('_us','_ms'):val(key)/1000 if val(key) is not None else None for key in ['worker_elapsed_us','admission_elapsed_us','admission_to_audio_queued_us']}})
(a.output/'raw.json').write_text(json.dumps(rows,indent=2)+'\n')
summary={gap:{'finished':len([r for r in rows if r['gap_ms']==gap]),**{key:{'count':len(values),'median':statistics.median(values),'p95':b.nearest_rank(values,.95),'maximum':max(values)} for key in ['worker_elapsed_ms','admission_elapsed_ms','admission_to_audio_queued_ms'] if (values:=[r[key] for r in rows if r['gap_ms']==gap and r[key] is not None])}} for gap in [100,50,30,20]}
(a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))
