"""Recompute retained responsiveness results from unmodified observations."""
import gzip,json,re,statistics
from pathlib import Path
root=Path(__file__).resolve().parent

def read(path):
 return path.read_text() if path.exists() else gzip.decompress(path.with_suffix(path.suffix+'.gz').read_bytes()).decode()

def stats(values):
 values=sorted(values)
 return dict(count=len(values),median_ms=statistics.median(values),p95_ms=values[max(0,(95*len(values)+99)//100-1)],maximum_ms=max(values))

summary={}
for target in ['before','after']:
 rows=[];runs=[]
 for block in range(2):
  d=root/f'{target}-letters-{block}'
  report=json.loads(read(d/'results.json'));rows+=report['samples']
  logs=read(d/'stderr.log')
  starts=logs.count('lifecycle_stage="mixer_source_started"')
  ends=logs.count('lifecycle_stage="mixer_source_ended"')
  assert starts==ends==len(report['settings']['letters'])*(report['settings']['iterations']+3),(d,starts,ends)
  runs.append(dict(name=d.name,starts=starts,ends=ends,pcm_waits=logs.count('lifecycle_stage="progressive_source_wait"')))
 summary[target]=dict(alphabet=stats([r['source_ms'] for r in rows if r['letter'].islower()]),
  worker=stats([r['worker_elapsed_ms'] for r in rows]),
  by_letter={letter:stats([r['source_ms'] for r in rows if r['letter']==letter]) for letter in sorted({r['letter'] for r in rows})},runs=runs)
 burst=root/f'{target}-bursts'
 summary[target]['bursts']=json.loads(read(burst/'results.json'))['summary']
 summary[target]['burst_pcm_waits']=read(burst/'stderr.log').count('lifecycle_stage="progressive_source_wait"')
 summary[target]['hard_stop_recoveries']=json.loads(read(burst/'results.json'))['hard_stop_recoveries']
lines={}
manifest=json.loads(read(root/'ordinary-lines/manifest.json'))
for job in manifest['jobs']:
 raw=json.loads(read(root/'ordinary-lines'/job['report']))
 key=job['target']+'/'+job['route']
 cell=lines.setdefault(key,{})
 for row in raw['timing_samples']:
  if row['case']!='line':continue
  for field in ['dispatch_to_source_ms','dispatch_to_terminal_ms']:
   if field in row:cell.setdefault(field,[]).append(row[field])
summary['ordinary_lines']={key:{field:stats(v) for field,v in cell.items()} for key,cell in lines.items()}
summary['custom_lines']=json.loads(read(root/'custom-lines/summary.json'))
slot=json.loads(read(root/'slot-results.json'))
summary['slot_wakeup']={k:stats([v/1e6 for run in m['runs'] for v in run]) for k,m in slot.items()}
summary['burst_repeats']={}
for target in ['before','after']:
 runs=[]
 for suffix in ['', '-1', '-2']:
  d=root/f'{target}-bursts{suffix}'
  data=json.loads(read(d/'results.json'))
  runs.append(dict(name=d.name,summary=data['summary'],last_source_ms=[data['source_ms_by_epoch'][str(i)] for i in [60,120,180,240,300]],hard_stop_recoveries=data['hard_stop_recoveries'],pcm_waits=read(d/'stderr.log').count('lifecycle_stage="progressive_source_wait"')))
 summary['burst_repeats'][target]=runs
summary['stop_grace_experiment']={}
for grace in (0,5):
 runs=[]
 for repeat in (0,1):
  d=root/f'grace-{grace}-{repeat}'
  data=json.loads(read(d/'results.json'))
  runs.append(dict(name=d.name,summary=data['summary'],last_source_ms=[data['source_ms_by_epoch'][str(i)] for i in [60,120,180,240,300]],hard_stop_recoveries=data['hard_stop_recoveries']))
 summary['stop_grace_experiment'][str(grace)]=runs
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
for target in ['before','after']:
 print(target,summary[target]['alphabet'],summary[target]['worker'],summary[target]['runs'])
 print('slow letters',{c:round(summary[target]['by_letter'][c]['median_ms'],3) for c in ['a','e','o','v','z','A']})
print(json.dumps(summary['ordinary_lines'],indent=2))
print(json.dumps(summary['custom_lines'],indent=2))
