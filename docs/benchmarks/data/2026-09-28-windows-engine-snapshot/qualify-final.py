from pathlib import Path
import json, os, subprocess, tempfile, time, shutil
out=Path(__file__).parent
repo=Path('/home/bart/src/omnivox')
emacsvox=Path('/home/bart/src/emacsvox')
runtime=emacsvox/'servers/omnivox-bin/current'
native_runtime=(runtime/'windows-runtime.path').read_text().strip()
server=Path(subprocess.check_output(['wslpath','-u',native_runtime],text=True).strip())/'omnivox.exe'
native_temp=subprocess.check_output(['powershell.exe','-NoProfile','-NonInteractive','-Command','[IO.Path]::GetTempPath()'],text=True).strip()
local_temp=Path(subprocess.check_output(['wslpath','-u',native_temp],text=True).strip())
results=[]
def run(name,command,cwd,environment):
 start=time.monotonic()
 with (out/(name+'.log')).open('w') as log:
  result=subprocess.run(command,cwd=cwd,env=environment,stdout=log,stderr=subprocess.STDOUT,timeout=500)
 row=dict(name=name,command=command,exit_code=result.returncode,elapsed_seconds=time.monotonic()-start)
 results.append(row)
 (out/'windows-qualification-result.json').write_text(json.dumps(dict(runtime=native_runtime,checks=results),indent=2)+'\n')
 print(json.dumps(row),flush=True)
 if result.returncode: print((out/(name+'.log')).read_text()[-3500:],flush=True)
 return result.returncode
base={k:v for k,v in os.environ.items() if not k.startswith('OMNIVOX_') and not k.startswith('EMACSVOX_')}
run('windows-framework-process-final',['python3','tools/verify_engine_configuration.py','--windows',str(server)],repo,base)
client_env=dict(base,EMACSVOX_ENGINE_FRAMEWORK_TEST_SERVER=str(server))
expr='(progn (setq temporary-file-directory '+json.dumps(str(local_temp)+'/')+') (dolist (fn (quote (tts-initialize omnivox-library--acknowledge-engines))) (unless (string-suffix-p ".elc" (symbol-file fn (quote defun))) (error "Expected compiled acceptance: %S" fn))) (ert-run-tests-batch-and-exit "omnivox-library-engine-"))'
run('windows-compiled-emacs',['/home/bart/opt/emacs-31/bin/emacs','-Q','--batch','-L','lisp','-l','lisp/emacsvox-preamble.el','-l','test/omnivox-library-tests.el','--eval',expr],emacsvox,client_env)
root=Path(tempfile.mkdtemp(prefix='omnivox-remote-qualified-',dir=local_temp))
remote_env=dict(base,OMNIVOX_REMOTE_TEST_PROGRAM=str(emacsvox/'servers/omnivox'),OMNIVOX_REMOTE_TEST_WINDOWS='1',OMNIVOX_REMOTE_TEST_SLOW='1',OMNIVOX_REMOTE_TEST_EMACS='/home/bart/opt/emacs-31/bin/emacs',OMNIVOX_REMOTE_TEST_EMACSVOX=str(emacsvox),OMNIVOX_REMOTE_TEST_COMPILED='1',OMNIVOX_REMOTE_TEST_EXPECT_ENGINE='espeak',OMNIVOX_ENGINE='espeak',OMNIVOX_VOICE_ROOT=str(root/'voices'),OMNIVOX_LOG_DIRECTORY=str(root/'logs'),EMACSVOX_OMNIVOX_CONFIG_FILE=str(root/'absent-launcher-config'))
status=run('windows-remote-compiled-emacs',['python3','tools/test_remote_service.py','-v'],repo,remote_env)
if not status: shutil.rmtree(root)
else: print('Retained isolated remote fixture directory: '+str(root),flush=True)
raise SystemExit(any(row['exit_code'] for row in results))
