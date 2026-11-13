#!/usr/bin/env python3
import json,math,statistics,sys
from pathlib import Path

def stats(xs):
    xs=sorted(xs)
    return dict(n=len(xs),median=statistics.median(xs),p95=xs[math.ceil(.95*len(xs))-1],maximum=xs[-1]) if xs else dict(n=0)
root=Path(sys.argv[1]);result={}
for work in ['ordered','letter']:
    result[work]={}
    for case in ['release','punctuation','host-idle','host-reload']:
        blocks=[json.loads((root/f'{work}-{i}-{case}'/'result.json').read_text()) for i in range(4)];assert all(b['success'] for b in blocks)
        overlap=[s['source_ms'] for b in blocks for s in b['samples'] if any(o.get('lane',0)==s.get('lane',0) and o['start_ns']<=s['start_ns']<=o['end_ns'] for o in b['operations'])]
        result[work][case]=dict(source_ms=stats([s['source_ms'] for b in blocks for s in b['samples']]),management_interval_source_ms=stats(overlap),reload_ms=stats([o['ms'] for b in blocks for o in b['operations']]),block_medians=[statistics.median(s['source_ms'] for s in b['samples']) for b in blocks],sha256=sorted(set(b['sha256'] for b in blocks)))
print(json.dumps(result,indent=2))
