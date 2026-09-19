import json,subprocess
from pathlib import Path
root=Path('/home/bart/src/emacsvox/.benchmarks/dectalk-letter-buffer-2026-09-20')
program='/mnt/c/Users/bart/AppData/Local/Emacsvox/Omnivox/runtime/6414dbbfa3e8543a/omnivox.exe'
manifest=[]
for name,script,args in [
 ('deployment-matched','measure-letters.py',[]),
 ('deployment-alphabet','measure-letters.py',['--letters','abcdefghijklmnopqrstuvwxyz','--iterations','5']),
 ('deployment-eloquence','measure-letters.py',['--engine','eloquence','--voice','v1','--iterations','8']),
 ('deployment-bursts','measure-bursts.py',[]),
]:
 cmd=['python3',str(root/script),str(root/name),'--program',program,*args]
 print('starting '+name,flush=True)
 with (root/(name+'.log')).open('w') as log:r=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
 manifest.append({'name':name,'command':cmd,'exit_code':r.returncode})
 (root/'deployment-matrix.json').write_text(json.dumps(manifest,indent=2)+'\n')
 if r.returncode:raise SystemExit('Failed '+name)
 print('passed '+name,flush=True)
