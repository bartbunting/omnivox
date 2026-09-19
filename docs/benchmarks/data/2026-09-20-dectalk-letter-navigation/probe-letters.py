import argparse,importlib.util,json,os,queue,re,time,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('output',type=Path);p.add_argument('--program',required=True);p.add_argument('--engine',default='dectalk');p.add_argument('--voice',default='paul');p.add_argument('--rate',default='225');a=p.parse_args()
a.output.mkdir(parents=True,exist_ok=False)
os.environ.update(OMNIVOX_PROGRAM=a.program,OMNIVOX_DECTALK_DLL='C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll',ESPEAK_NG_DATA='C:/Users/bart/AppData/Local/Emacsvox/Omnivox/espeak-data/9a2e176369cfe0b982737abf6c26037f21d2b9af2a2fabcc08062864d0157718',RUST_LOG='info',EMACSVOX_OMNIVOX_DIAGNOSTIC='1',OMNIVOX_LOG_DIRECTORY=str(a.output/'logs'))
spec=importlib.util.spec_from_file_location('b','/home/bart/src/omnivox/tools/benchmark_server.py');b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class Session(b.ServerSession):
 def __init__(self):
  self.events=queue.Queue();self.logs=[]
  super().__init__(['/home/bart/src/emacsvox/servers/omnivox','--audio-output','null'],a.engine,20)
 def _read_stderr(self):
  for line in self.process.stderr:
   self.logs.append(line)
   if 'lifecycle_stage="worker_finished"' in line and ('request_kind="letter"' in line or 'request_kind="immediate"' in line):self.events.put(line)
 def measure(self,command):
  self.send_line(command)
  line=self.events.get(timeout=20)
  fields={k.replace("_us", "_ms"): int(m.group(1))/1000 if (m:=re.search(k+r'=(?:Some\()?([0-9]+)',line)) else None for k in ['worker_elapsed_us','admission_elapsed_us','admission_to_audio_queued_us']}
  return {'command':command,'timings_ms':fields,'log':line.strip()}
s=Session();rows=[]
try:
 cap,_=s.negotiate(10000)
 b.configure_preferred_engine(s,cap,a.engine,10001)
 s.send_line('tts_set_voice '+a.voice);s.send_line('tts_set_speech_rate '+a.rate);s.send_line('tts_set_character_scale 1.0')
 for command in ['l {a}','tts_say {a}','l {A}','l {b}','l {w}']:
  for i in range(23):
   r=s.measure(command)
   if i>=3:rows.append(r)
finally:
 s.close();(a.output/'stderr.log').write_text(''.join(s.logs))
(a.output/'raw.json').write_text(json.dumps(rows,indent=2)+'\n')
summary={c:{k:statistics.median([r['timings_ms'][k] for r in rows if r['command']==c and r['timings_ms'][k] is not None]) for k in rows[0]['timings_ms']} for c in sorted({r['command'] for r in rows})}
(a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))
