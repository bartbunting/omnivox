import argparse, copy, hashlib, importlib.util, json, random, subprocess, sys
from pathlib import Path
root=Path('/home/bart/src/emacsvox')
spec=importlib.util.spec_from_file_location('benchmark',root/'utils/voice_benchmark.py')
b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
parser=argparse.ArgumentParser()
parser.add_argument('output');parser.add_argument('--target',action='append',required=True,help='name=helper.exe; use stock for default');parser.add_argument('--iterations',type=int,default=10);parser.add_argument('--engines',default='eloquence,espeak,dectalk');parser.add_argument('--repeats',type=int,default=1)
a=parser.parse_args();out=Path(a.output).resolve();out.mkdir(parents=True,exist_ok=False)
plan=json.loads((root/'.benchmarks/voices.json').read_text());template=plan['targets'][1]
manifest={'kind':'focused line diagnosis; software onset, no acoustic onset','targets':{},'jobs':[], 'harness':{str(p):b.digest(p) for p in [root/'utils/voice_benchmark.py',root/'utils/voice_benchmark_server.py',Path(plan['omnivox_root'])/'tools/benchmark_server.py']}}
targets=[]
for arg in a.target:
 name,helper=arg.split('=',1);t=copy.deepcopy(template);program=t['program'];t.update(id=name,native=False,ui=False)
 if helper != 'stock':
  t['environment']['OMNIVOX_DECTALK_HELPER']=subprocess.check_output(['wslpath','-w',helper],text=True).strip()
 targets.append(t)
 manifest['targets'][name]={'program':program,'sha256':b.digest(program),'payload':{p.name:b.digest(p) for p in Path(program).parent.iterdir() if p.is_file() and p.suffix.lower() in ('.exe','.dll')}, 'provenance': (Path(program).parent/'PROVENANCE').read_text() if (Path(program).parent/'PROVENANCE').exists() else None}
 if helper != 'stock': manifest['targets'][name]['helper']={'path':helper,'sha256':b.digest(helper)}
for repeat in range(a.repeats):
 jobs=[(t,r) for t in targets for r in plan['routes'] if r['engine'] in a.engines.split(',')];random.Random(71423+repeat).shuffle(jobs)
 for t,r in jobs:
  name=f"{len(manifest['jobs']):02}-{t['id']}-{r['id']}";d=out/name;d.mkdir()
  job={'audio_output':'null','cases':['character','word','line','dense','multipart','replacement'],'concurrent':True,'flavour':'legacy','iterations':a.iterations,'mode':'warm','omnivox_root':plan['omnivox_root'],'rate':225,'replacement_burst':5,'route':r,'server':t['server'],'timeout':30,'warmups':3}
  b.write_json(d/'job.json',job);print(name,flush=True)
  b.run_child([sys.executable,str(root/'utils/voice_benchmark_server.py'),str(d/'job.json'),str(d/'raw.json')],b.child_environment(t,d),d/'worker.log',max(120,a.iterations*30))
  raw=json.loads((d/'raw.json').read_text());vals=[s['dispatch_to_source_ms'] for s in raw['timing_samples'] if s['case']=='line']
  entry={'target':t['id'],'route':r['id'],'repeat':repeat,'report':str((d/'raw.json').relative_to(out)),'sha256':b.digest(d/'raw.json'),'n':len(vals),'median':b.percentile(vals,.5),'p95':b.percentile(vals,.95)}
  manifest['jobs'].append(entry);b.write_json(out/'manifest.json',manifest);print(entry,flush=True)
