"""Summarize native DECtalk call intervals; phases are inside one owner clock."""
from pathlib import Path
import gzip,json,math,statistics,sys
root=Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).parent
p=root/'parameter-timings.json'
data=p.read_bytes() if p.exists() else gzip.decompress((root/'parameter-timings.json.gz').read_bytes())
j=json.loads(data.decode('utf-8-sig'))
def stats(v):
 v=sorted(v)
 return dict(n=len(v),median_ms=statistics.median(v),p95_ms=v[math.ceil(len(v)*.95)-1])
summary={}
for scenario in ['ordinary','native_empty','native_one','native_all']:
 samples=[s for s in j['samples'] if s['case']==scenario]
 assert len(samples)==90,len(samples)
 def total(s,api=None,phase=None,preparation=False):
  return sum(c['duration_ms'] for c in s['calls'] if (api is None or c['api']==api) and (phase is None or c['phase']==phase) and (not preparation or c['phase'] in ['preset','common','edits']))
 row={'total':stats([s['total_ms'] for s in samples]),'preparation_sync':stats([total(s,'sync',preparation=True) for s in samples]),
 'preparation_speak':stats([total(s,'speak',preparation=True) for s in samples]),'preparation_readback':stats([total(s,'readback',preparation=True) for s in samples]),
 'preparation_sync_counts':sorted(set(sum(c['api']=='sync' and c['phase'] in ['preset','common','edits'] for c in s['calls']) for s in samples))}
 if scenario!='ordinary':
  row['applied']=stats([s['applied_ms'] for s in samples])
  row['remaining_preparation']=stats([s['applied_ms']-total(s,'sync',preparation=True) for s in samples])
 for phase in ['preset','common','edits','speech','restore']:
  times=[c['duration_ms'] for s in samples for c in s['calls'] if c['phase']==phase and c['api']=='sync']
  if times:row[phase+'_sync']=stats(times)
 summary[scenario]=row
(root/'parameter-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
