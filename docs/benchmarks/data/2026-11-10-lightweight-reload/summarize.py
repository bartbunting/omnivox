#!/usr/bin/env python3
"""Summarize retained JSON after extracting observations.tar.gz."""
import json,math,statistics,sys
from pathlib import Path
root=Path(sys.argv[1])
def stats(rows):
 a=sorted(rows)
 return dict(n=len(a),median=statistics.median(a),p95=a[math.ceil(len(a)*.95)-1],maximum=a[-1]) if a else None
result={}
for name in ['matched-ordered-2','matched-letter','matched-replacement','overlap-1']:
 cases={}
 for p in sorted((root/name).glob('*/result.json')):
  v=json.loads(p.read_text());assert v['success']
  case=p.parent.name.split('-',1)[1];out=cases.setdefault(case,dict(samples=[],operations=[],preparation=[],runs=[],during=[],overlap=[]))
  out['samples'] += [x['source_ms'] for x in v['samples']]
  out['operations'] += [x['ms'] for x in v['operations']]
  if 'prepare_ms' in v:out['preparation'].append(v['prepare_ms'])
  out['runs'].append(dict(run=p.parent.name,**stats([x['source_ms'] for x in v['samples']])))
  ops=[x for x in v['operations'] if 'lane' in x and 'start_ns' in x]
  if ops:
   for s in v['samples']:
    start=s['start_ns'];end=start+s['source_ms']*1e6
    if ops[0]['start_ns']<=start<=ops[-1]['end_ns']:out['during'].append(s['source_ms'])
    if any(x['lane']==s['lane'] and x['start_ns']<=end and x['end_ns']>=start for x in ops):out['overlap'].append(s['source_ms'])
 result[name]={c:{k:(v if k=='runs' else stats(v)) for k,v in d.items()} for c,d in cases.items()}
print(json.dumps(result,indent=2))
