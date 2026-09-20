import gzip,hashlib,json,subprocess
from pathlib import Path
root=Path(__file__).resolve().parent
repo=Path('/home/bart/src/omnivox')
dest=repo/'docs/benchmarks/data/2026-09-20-responsiveness'
dest.mkdir(parents=True,exist_ok=False)
records={}
include_dirs=[p for p in root.iterdir() if p.is_dir() and (p.name.startswith(('before-letters','after-letters','before-bursts','after-bursts','grace-')) or p.name in ('ordinary-lines','custom-lines','after-slow','after-fast','timer-device'))]
files=[p for p in root.iterdir() if p.is_file() and (p.suffix in ('.py','.json','.log','.stderr','.patch','.rs','.txt') or p.name.endswith(('PROVENANCE','SHA256SUMS')))]
for d in include_dirs:files += [p for p in d.rglob('*') if p.is_file() and p.suffix in ('.json','.log')]
for p in sorted(files):
 rel=p.relative_to(root);data=p.read_bytes()
 packed=p.suffix in ('.log','.stderr','.patch','.rs') or b'\r\n' in data or (p.suffix=='.json' and len(data)>200_000)
 if packed:rel=rel.with_suffix(rel.suffix+'.gz')
 target=dest/rel;target.parent.mkdir(parents=True,exist_ok=True)
 target.write_bytes(gzip.compress(data,mtime=0) if packed else data)
 records[str(rel)]={'original_path':str(p),'original_sha256':hashlib.sha256(data).hexdigest(),'original_bytes':len(data),'stored_sha256':hashlib.sha256(target.read_bytes()).hexdigest()}
inputs={}
for path in [repo/'omnivox-cli/src/engine_execution.rs',repo/'windows-helpers/dectalk/OmnivoxDectalkCapture.cs',repo/'tools/DectalkExecutionAudit.cs',repo/'tools/test_helper6_dectalk.py',repo/'tools/stress_helper.py',repo/'tools/benchmark_server.py',Path('/home/bart/src/emacsvox/utils/voice_benchmark.py'),Path('/home/bart/src/emacsvox/utils/voice_benchmark_server.py'),Path('/home/bart/src/emacsvox/.benchmarks/dectalk-reset-implementation-2026-09-20/run-server-lines.py'),Path('/home/bart/src/dectalk/src/dapi/src/api/ttsapi.c'),Path('/mnt/c/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/DECtalk.dll'),Path('/mnt/c/Users/bart/AppData/Local/Omnivox/runtimes/dectalk/x86/dtalk_us.dic'),root/'timer-probe/bin/OmnivoxDectalkHelper32.exe',root/'stop-probe/bin/OmnivoxDectalkHelper32.exe']:
 inputs[str(path)]={'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'bytes':path.stat().st_size}
index={'source_commit':'bd95655b2e0ec729b5482189e7906057056c0ce1','emacsvox_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd='/home/bart/src/emacsvox',text=True).strip(),'dectalk_source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd='/home/bart/src/dectalk',text=True).strip(),'before_runtime':'6414dbbfa3e8543a','after_runtime':'577414114415d7b0','full_candidate_base':'626ad994ff49f015','measurement_limits':['software timing; muted device or null backend','private helper timer experiment has exact matched PCM/marker sequences','native macOS and acoustic onset not measured','stop-grace experiment not deployed'],'observed_inputs':inputs,'files':records}
(dest/'index.json').write_text(json.dumps(index,indent=2)+'\n')
lines=[]
for p in sorted(dest.rglob('*')):
 if p.is_file():lines.append(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+str(p.relative_to(dest)))
(dest/'SHA256SUMS').write_text('\n'.join(lines)+'\n')
print(len(records),'files retained; stored bytes',sum(p.stat().st_size for p in dest.rglob('*') if p.is_file()))
