from pathlib import Path
import subprocess,json,time
p=Path(Path('/tmp/omnivox-current-onset-fix').read_text());root=Path('/mnt/c/Users/bart/AppData/Local/Temp')/(p.name+'-captures');root.mkdir(exist_ok=False)
(p/'capture-root.txt').write_text(str(root))
runner=Path('docs/benchmarks/data/2026-09-28-streaming-onset-loss/reproduce_windows.py').resolve()
for label in ['baseline','fixed']:
 runtime=Path('/mnt/c/Users/bart/AppData/Local/Temp')/(p.name+'-'+label)
 # Wait only for the staging build to finish, outside any user-facing tool wait.
 deadline=time.monotonic()+120
 while True:
  records=json.loads((p/'windows-builds.json').read_text())
  if any(r['label']==label for r in records):break
  if time.monotonic()>deadline:raise RuntimeError('test payload staging timed out')
  time.sleep(1)
 command=['python3',str(runner),'--omnivox',str(runtime/'omnivox.exe'),'--output',str(root/label)]
 if label=='baseline':command+=['--case','mono22050','--case','mono16000','--case','lead']
 with (p/(label+'-capture.log')).open('w') as log:
  subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
 print((p/(label+'-capture.log')).read_text(),flush=True)
print('capture root',root,flush=True)
