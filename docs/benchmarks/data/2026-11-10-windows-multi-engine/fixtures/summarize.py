#!/usr/bin/env python3
"""Summarize retained source observations; no acoustic or tail-latency claim."""
import json, math, statistics as s, sys
from pathlib import Path
root=Path(sys.argv[1]); output=Path(sys.argv[2]);result={}
def read(name): return json.loads((root/('omnivox-multi-20261110-'+name)/'result.json').read_text())
def stats(v):return dict(n=len(v),median=s.median(v),min=min(v),max=max(v),empirical_p95=sorted(v)[math.ceil(.95*len(v))-1])
latency=read('latency-final');result['speech']={}
for engine in latency['runs'][0]['engines']:
 rows=[]
 for guarded in (False,True):
  blocks=[r['engines'][engine] for r in latency['runs'] if r['guarded']==guarded]
  v=[x['source_ms'] for b in blocks for x in b['samples']]
  rows.append(dict(guarded=guarded,source_ms=stats(v),block_medians=[b['median_ms'] for b in blocks]))
 result['speech'][engine]=rows
result['interaction']=[]
for r in read('interaction-final')['runs']:
 assert r['passed']
 result['interaction'].append(dict(engine=r['engine'],guarded=r['guarded'],stop_ms=stats([v['cancellation_ms'] for v in r['stop']]),after_stop_source_ms=stats([v['replacement_source_ms'] for v in r['stop']]),replacement_source_ms=stats([v['replacement_source_ms'] for v in r['replace']]),old_outcomes=sorted(set(v['old_outcome'] for k in ('stop','replace') for v in r[k]))))
result['startup']={}
for variant in ('baseline','candidate'):
 runs=sorted(root.glob('omnivox-multi-20261110-startup-*-'+variant));rows=[r for p in runs for r in json.loads((p/'startup-results.json').read_text()) if r['operation']=='restart'];spans=[r for p in runs for r in json.loads((p/'startup-spans.json').read_text())]
 result['startup'][variant]=dict(operation_ms=stats([r['operation_ms'] for r in rows]),first_source_ms=stats([r['first_ms'] for r in rows]),spans={fn:stats([r['ms'] for r in spans if r['function']==fn]) for fn in sorted(set(r['function'] for r in spans))})
result['availability']={}
for variant in ('baseline','candidate'):
 trials=json.loads((root/f'omnivox-multi-20261110-restart-availability2-{variant}'/'restart-availability.json').read_text());out=[]
 for trial in trials:
  row=dict(trial=trial['trial'],operation_ms=trial['operation_ms'],samples=len(trial['samples']),cancelled=sum(x['terminal']=='cancelled' for x in trial['samples']))
  for lane in ('main','notification'):
   old=[x['first'] for x in trial['samples'] if x['lane']==lane and x['old'] and x['first']];new=[x['first'] for x in trial['samples'] if x['lane']==lane and not x['old'] and x['first']]
   row[lane+'_handover_source_gap_ms']=1000*(min(new)-max(old))
  out.append(row)
 result['availability'][variant]=out
output.write_text(json.dumps(result,indent=2)+'\n')
for e,rows in result['speech'].items():print(e,' / '.join(f"{r['source_ms']['median']:.2f} ({r['source_ms']['empirical_p95']:.2f})" for r in rows))
print('interaction stop medians range',min(r['stop_ms']['median'] for r in result['interaction']),max(r['stop_ms']['median'] for r in result['interaction']))
