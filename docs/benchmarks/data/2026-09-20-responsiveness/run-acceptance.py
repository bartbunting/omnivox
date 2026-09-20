"""Serial matched device and shared-path checks after staging completes."""
import hashlib,json,shutil,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parent
evox=Path('/home/bart/src/emacsvox'); ovx=Path('/home/bart/src/omnivox')
current=(root/'staged-runtime/current').resolve(strict=True)
assert (current/'PROVENANCE').is_file()
windows=Path((current/'windows-runtime.path').read_text().strip())
# windows-runtime.path contains the WSL executable directory used by staging.
print('staged path',windows,flush=True)
if not windows.is_dir():
 windows=Path(subprocess.check_output(['wslpath','-u',str(windows)],text=True).strip())
programs={'before':Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/Omnivox/runtime/6414dbbfa3e8543a/omnivox.exe'),'after':windows/'omnivox.exe'}
shutil.copy2(evox/'.benchmarks/voices.json',root/'voices-plan-used.json')
manifest={'programs':{k:str(v) for k,v in programs.items()},'jobs':[]}
for name,program in programs.items():
 assert program.is_file(),program
 for file in ['PROVENANCE','SHA256SUMS']:
  shutil.copy2(program.parent/file,root/(name+'-'+file))
manifest['hashes']={name:{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in program.parent.iterdir() if p.is_file() and p.suffix.lower() in ('.exe','.dll')} for name,program in programs.items()}
def run(name,args):
 print('starting',name,flush=True)
 with (root/(name+'.log')).open('w') as log:
  r=subprocess.run([sys.executable,*map(str,args)],cwd=evox,stdout=log,stderr=subprocess.STDOUT)
 manifest['jobs'].append({'name':name,'command':[sys.executable,*map(str,args)],'exit_code':r.returncode})
 (root/'acceptance-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 if r.returncode:raise RuntimeError('failed '+name)
 print('passed',name,flush=True)
run('slot-wakeup',[root/'measure-slot-wakeup.py'])
letters=ovx/'docs/benchmarks/data/2026-09-20-letter-playback-reserve/measure-letters.py'
bursts=ovx/'docs/benchmarks/data/2026-09-20-letter-playback-reserve/measure-bursts.py'
# Reverse the second block to avoid making every after measurement later.
for block in range(2):
 for target in (['before','after'] if block==0 else ['after','before']):
  name=f'{target}-letters-{block}'
  run(name,[letters,root/name,'--program',programs[target],'--letters','abcdefghijklmnopqrstuvwxyzA','--iterations','4','--gap-ms','300'])
for target in ['before','after']:
 name=f'{target}-bursts'
 run(name,[bursts,root/name,'--program',programs[target]])
for name,extra in [('after-slow',['--rate','50']),('after-fast',['--rate','150'])]:
 run(name,[letters,root/name,'--program',programs['after'],'--iterations','6',*extra])
# Common-path compatibility and ordinary/custom DECtalk measurements use the maintained harness.
lines=evox/'.benchmarks/dectalk-reset-implementation-2026-09-20/run-server-lines.py'
run('ordinary-lines',[lines,root/'ordinary-lines','--target','before='+str(programs['before']),'--target','after='+str(programs['after']),'--iterations','30','--repeats','3'])
custom=ovx/'docs/benchmarks/data/2026-09-20-dectalk-batched-parameters/compare-server.py'
run('custom-lines',[custom,root/'custom-lines','--emacsvox',evox,'--plan',evox/'.benchmarks/voices.json','--target','before='+str(programs['before']),'--target','after='+str(programs['after']),'--iterations','20','--repeats','2'])
print('all comparisons passed',flush=True)
