from pathlib import Path
import subprocess,os,shutil,hashlib,json
p=Path(Path('/tmp/omnivox-current-onset-fix').read_text());s=p/'release-source'
env=dict(os.environ,RUSTUP_TOOLCHAIN='1.97.1',CARGO_TARGET_DIR=str(p/'windows-target'))
# The same staging wrapper used by make dev retains generated eSpeak data and notices.
command=['cargo','+1.97.1','build','--locked','--package','omnivox-cli','--target','x86_64-pc-windows-gnu']
runtime=Path(Path('/tmp/omnivox-current-loopback').read_text())/'release'
records=[]
for label in ['baseline','fixed']:
 if label=='fixed':shutil.copyfile(p/'fixed-progressive_pcm.rs',s/'omnivox-audio/src/progressive_pcm.rs')
 with (p/(label+'-windows-complete-build.log')).open('w') as log:
  subprocess.run(command,cwd=s,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 profile=p/'windows-target/x86_64-pc-windows-gnu/debug'
 dest=Path('/mnt/c/Users/bart/AppData/Local/Temp')/(p.name+'-'+label);dest.mkdir(exist_ok=False)
 # Reuse the exact published 1.14.0 runtime data and notices for this isolated
 # test package. Cross-compilation generates no eSpeak data; main-only output
 # is not treated as a complete distributable. No user runtime is replaced.
 shutil.copytree(runtime,dest,dirs_exist_ok=True)
 shutil.copyfile(profile/'omnivox.exe',dest/'omnivox.exe')
 for name in ['libgcc_s_seh-1.dll','libstdc++-6.dll']:
  dll=Path('/home/bart/src/omnivox/target/x86_64-pc-windows-gnu/release')/name
  shutil.copyfile(dll,dest/name)
 records.append({'label':label,'directory':str(dest),'sha256':hashlib.sha256((dest/'omnivox.exe').read_bytes()).hexdigest(),'command':command,'toolchain':'1.97.1','runtime_assets':'published Windows x64 1.14.0 archive; main replaced for isolated test only','source':'v1.14.0' if label=='baseline' else 'v1.14.0 plus retained fix.patch'})
 (p/'windows-builds.json').write_text(json.dumps(records,indent=2)+'\n')
 print(label,dest,flush=True)
