import base64, hashlib, json, os, sys, time
from pathlib import Path
sys.path.insert(0, '/home/bart/src/omnivox/tools')
from test_helper6_eloquence import Session
root=Path(__file__).resolve().parent
program=root/'timer-probe/bin/OmnivoxDectalkHelper32.exe'
os.environ['WSLENV']=os.environ.get('WSLENV','')+':OMNIVOX_PROBE_TIMER'
rows=[]
for block, enabled in enumerate([False,True,True,False]):
 os.environ['OMNIVOX_PROBE_TIMER']='1' if enabled else '0'
 session=Session(program,'C:/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll')
 try:
  for text in ['a','b','w','A','A short line for the timing comparison.']:
   for i in range(23):
    started=time.perf_counter_ns()
    message=session.send('synthesize',text=text,settings=dict(voice_id='paul',rate=.5,pitch=1.0,pitch_range=None,stress=None,richness=None,volume=1.0),anchors=[],voice_parameters=None)
    events=[];pcm=bytearray();markers=[]
    while True:
     frame=session.frames.get(timeout=15)
     elapsed=(time.perf_counter_ns()-started)/1e6
     assert frame is not None and frame['request_id']==message['request_id'],frame
     events.append([frame['type'],elapsed])
     if frame['type']=='audio_chunk':pcm.extend(base64.b64decode(frame['chunk']['data_base64']))
     if frame['type']=='markers':markers.extend(frame['markers'])
     if frame['type'] in ('synthesis_completed','synthesis_cancelled','error'):break
    assert frame['type']=='synthesis_completed' and frame['frame_count']==len(pcm)//2,frame
    if i>=3:
     rows.append(dict(block=block,timer=enabled,text=text,events=events,frames=frame['frame_count'],pcm_sha256=hashlib.sha256(pcm).hexdigest(),markers=markers))
 finally:session.close()
 (root/'timer-results.json').write_text(json.dumps(rows,indent=2)+'\n')
 print('completed',block,enabled,flush=True)
import statistics
for text in sorted({r['text'] for r in rows}):
 for enabled in (False,True):
  rs=[r for r in rows if r['text']==text and r['timer']==enabled]
  print(text,enabled,{key:statistics.median([([e[1] for e in r['events'] if e[0]=='audio_chunk'][0 if key=='first' else -1] if key!='terminal' else r['events'][-1][1]) for r in rs]) for key in ['first','last','terminal']},'hashes',len({r['pcm_sha256'] for r in rs}))
