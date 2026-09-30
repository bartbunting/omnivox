#!/usr/bin/env python3
from pathlib import Path
import numpy as np,json,struct,argparse
parser=argparse.ArgumentParser(description='Align retained process-loopback PCM against its reference WAV.')
parser.add_argument('--directory',type=Path,required=True)
p=parser.parse_args().directory
def wav(path):
 b=path.read_bytes();i=12;chunks={}
 while i+8<=len(b):
  k=b[i:i+4];n=struct.unpack_from('<I',b,i+4)[0];chunks[k]=b[i+8:i+8+n];i+=8+n+(n%2)
 fmt=chunks[b'fmt '];tag,ch,rate=struct.unpack_from('<HHI',fmt)
 assert rate==44100,(path,rate)
 return np.frombuffer(chunks[b'data'],dtype='<f4' if tag==3 else '<i2').reshape(-1,ch)[:,0].astype(float)/(1 if tag==3 else 32768)
results={}
files=sorted(list(p.glob('gap-*.f32'))+list(p.glob('wav-*.f32'))+list(p.glob('helper-*-fast.f32'))+list(p.glob('helper-*-lead.f32')))
for f in files:
 label=f.stem
 if label.startswith('wav-'):
  reference=wav(p/(label[4:]+'.wav'));nzr=np.flatnonzero(abs(reference)>1e-5);reference=reference[nzr[0]:nzr[-1]+1]
 else:reference=wav(p/(label+'.reference.wav'));nzr=np.flatnonzero(abs(reference)>1e-5);reference=reference[nzr[0]:nzr[-1]+1]
 anchor_start=1800 if ('lead' in label) else 1000;anchor=reference[anchor_start:anchor_start+3000]
 x=np.fromfile(f,'<f4').reshape(-1,2)[:,0];nz=np.flatnonzero(abs(x)>1e-5)
 if not len(nz):raise RuntimeError('No audio '+label)
 cut=np.flatnonzero(np.diff(nz)>1000);starts=np.r_[nz[0],nz[cut+1]];ends=np.r_[nz[cut]+1,nz[-1]+1]
 matches=[];excluded=[]
 for a,b in zip(starts,ends):
  y=x[a:b]
  if len(y)<len(anchor):excluded.append([int(a),int(b)]);continue
  at=int(np.argmax(np.correlate(y,anchor-anchor.mean(),'valid')));matched=y[at:at+len(anchor)]
  corr=float(np.corrcoef(matched,anchor)[0,1])
  if corr<.99:excluded.append([int(a),int(b)]);continue
  offset=at-anchor_start;gain=float(np.dot(matched,anchor)/np.dot(anchor,anchor));err=float(np.sqrt(np.mean((matched-anchor*gain)**2)))
  matches.append({'captured_start':int(a),'captured_frames':int(b-a),'reference_frames':len(reference),'reference_onset_relative_to_capture':offset,'missing_onset_frames':max(0,-offset),'missing_ms':max(0,-offset)/44.1,'correlation':corr,'gain':gain,'body_rmse':err})
 packets=np.genfromtxt(str(f)+'.packets.csv',delimiter=',',names=True)
 results[label]={'matches':matches,'excluded_nonmatching_segments':excluded,'capture_frames':len(x),'packet_flags':np.unique(packets['flags']).astype(int).tolist()}
 print(label,'N',len(matches),'loss',[m['missing_onset_frames'] for m in matches],'corr',round(min((m['correlation'] for m in matches),default=0),6))
(p/'analysis-final.json').write_text(json.dumps(results,indent=2))
