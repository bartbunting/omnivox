#!/usr/bin/env python3
"""Summarize retained resident measurements; empirical nearest-rank percentiles."""
import json, math, statistics, sys
from pathlib import Path

def distribution(values):
    values = sorted(values)
    if not values:
        return {"n": 0}
    return dict(n=len(values), median=statistics.median(values),
                p95=values[math.ceil(.95*len(values))-1], maximum=values[-1])

def summarize(root):
    result = {}
    for workload in ['ordered', 'letter', 'replaceable']:
        result[workload] = {}
        for case in ['release', 'maintenance', 'resident-idle', 'resident-reload']:
            blocks = [json.loads((root/f'{workload}-{i}-{case}'/'result.json').read_text()) for i in range(8)]
            assert all(b['success'] for b in blocks)
            samples = [s['source_ms'] for b in blocks for s in b['samples']]
            overlaps = []
            for b in blocks:
                for s in b['samples']:
                    lane = s.get('lane', 0)
                    if any(o.get('lane', 0) == lane and o['start_ns'] <= s['start_ns'] <= o['end_ns'] for o in b['operations']):
                        overlaps.append(s['source_ms'])
            result[workload][case] = dict(source_ms=distribution(samples),
                block_medians=[statistics.median(s['source_ms'] for s in b['samples']) for b in blocks],
                management_interval_source_ms=distribution(overlaps),
                reload_ms=distribution([o['ms'] for b in blocks for o in b['operations']]),
                two_ready_ms=distribution([b['two_ready_ms'] for b in blocks]),
                sha256=sorted(set(b['sha256'] for b in blocks)))
    return result

if __name__ == '__main__':
    print(json.dumps(summarize(Path(sys.argv[1])), indent=2))
