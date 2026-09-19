import json,subprocess,time
from pathlib import Path
root=Path('/home/bart/src/emacsvox/.benchmarks/dectalk-letter-buffer-2026-09-20')
programs={k:'/mnt/c/Users/bart/AppData/Local/Emacsvox/Omnivox/runtime/'+v+'/omnivox.exe' for k,v in [('baseline','f8b3b04d3243ee76'),('final','d214e1ecc77ec84f')]}
jobs=[('final','matched',[]),('baseline','matched',[]),('baseline','alphabet',['--letters','abcdefghijklmnopqrstuvwxyz','--iterations','5']),('final','alphabet',['--letters','abcdefghijklmnopqrstuvwxyz','--iterations','5']),('final','slow',['--rate','50','--iterations','8','--gap-ms','450']),('final','fast',['--rate','150','--iterations','8']),('baseline','eloquence',['--engine','eloquence','--voice','v1','--iterations','8']),('final','eloquence',['--engine','eloquence','--voice','v1','--iterations','8'])]
manifest=[]
for target,label,args in jobs:
 name=target+'-'+label
 cmd=['python3',str(root/'measure-letters.py'),str(root/name),'--program',programs[target],*args]
 print('starting '+name,flush=True);start=time.time()
 with (root/(name+'.log')).open('w') as log:r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
 manifest.append({'name':name,'command':cmd,'elapsed_seconds':time.time()-start,'exit_code':r.returncode})
 (root/'matrix.json').write_text(json.dumps(manifest,indent=2)+'\n')
 if r.returncode:raise SystemExit('Failed '+name)
 print('passed '+name,flush=True)
cmd=['python3',str(root/'measure-bursts.py'),str(root/'final-bursts'),'--program',programs['final']]
print('starting final-bursts',flush=True)
with (root/'final-bursts.log').open('w') as log:r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
manifest.append({'name':'final-bursts','command':cmd,'exit_code':r.returncode});(root/'matrix.json').write_text(json.dumps(manifest,indent=2)+'\n')
raise SystemExit(r.returncode)
