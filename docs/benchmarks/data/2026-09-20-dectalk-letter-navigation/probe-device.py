import argparse,importlib.util,json,os,statistics
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('output',type=Path);p.add_argument('--program',required=True);p.add_argument('--backend',choices=['device','null'],required=True);p.add_argument('--rate',default='85');a=p.parse_args()
a.output.mkdir(parents=True,exist_ok=False)
os.environ.update(OMNIVOX_PROGRAM=a.program,OMNIVOX_DECTALK_DLL='C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll',ESPEAK_NG_DATA='C:/Users/bart/AppData/Local/Emacsvox/Omnivox/espeak-data/9a2e176369cfe0b982737abf6c26037f21d2b9af2a2fabcc08062864d0157718',RUST_LOG='info',EMACSVOX_OMNIVOX_DIAGNOSTIC='1',OMNIVOX_LOG_DIRECTORY=str(a.output/'logs'))
spec=importlib.util.spec_from_file_location('b','/home/bart/src/omnivox/tools/benchmark_server.py');b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class Session(b.ServerSession):
 def _read_stderr(self):
  for line in self.process.stderr:self.stderr_lines.append(line)
s=Session(['/home/bart/src/emacsvox/servers/omnivox','--audio-output',a.backend],'dectalk',20);rows=[];ids=b.IdentitySequence()
try:
 cap,_=s.negotiate(10000);b.configure_preferred_engine(s,cap,'dectalk',10001)
 s.send_line('tts_set_voice paul');s.send_line('tts_set_speech_rate '+a.rate);s.send_line('tts_set_voice_volume 0')
 for text in ['a','b','w','latency']:
  for i in range(23):
   generation,identifier=ids.next()
   timeline={'protocol_version':3,'generation':generation,'dispatch_id':identifier,'delivery_policy':'ordered','spans':[{'id':1,'text':text}],'actions':[]}
   start=s.send_timeline(timeline);r=s.wait_for_dispatches({identifier})[identifier];b.require_completed(r,identifier)
   assert r['engine_id']=='dectalk',r
   if i>=3:rows.append({'text':text,'dispatch_id':identifier,'source_ms':b.milliseconds(r['source_at_ns'],start),'terminal_ms':b.milliseconds(r['terminal_at_ns'],start),'engine_id':r['engine_id']})
finally:
 s.close();(a.output/'stderr.log').write_text(''.join(s.stderr_lines))
(a.output/'raw.json').write_text(json.dumps(rows,indent=2)+'\n')
summary={text:{k:statistics.median([r[k] for r in rows if r['text']==text]) for k in ['source_ms','terminal_ms']} for text in sorted({r['text'] for r in rows})}
(a.output/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))
