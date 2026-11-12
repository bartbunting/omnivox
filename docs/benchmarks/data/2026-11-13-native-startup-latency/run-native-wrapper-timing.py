import subprocess,base64
from pathlib import Path
p=Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/NativeLightweight/native-early-20261113/startup-timing-20261113')
final=(p/'evox-windows.ps1').read_text()
# The baseline uses exactly the former two preflight subprocesses.
baseline=final.replace("if ($Check) {\n    $selected = Get-EmacsvoxNativeEmacs $config.Emacs", "if ($true) {\n    $selected = Get-EmacsvoxNativeEmacs $config.Emacs",1)
for run,script,engine in [('wrapper-before-1',baseline,'eloquence'),('wrapper-after-1',final,'eloquence'),('wrapper-before-2',baseline,'eloquence'),('wrapper-after-2',final,'eloquence'),('wrapper-dectalk',final,'dectalk')]:
 (p/'evox-windows.ps1').write_text(script)
 s=f'''$ErrorActionPreference='Stop'
 $root='C:\\Users\\bart\\AppData\\Local\\Emacsvox\\NativeLightweight\\native-early-20261113\\startup-timing-20261113'
 $env:EVOX_TIMING_RUN='{run}'; $env:OMNIVOX_ENGINE='{engine}'
 $start=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
 $process=[Diagnostics.Process]::Start((Join-Path $root 'evox-windows-launcher.exe'))
 if (-not $process.WaitForExit(20000)) {{throw 'Wrapper did not exit'}}
 if ($process.ExitCode -ne 0) {{throw 'Wrapper failed'}}
 $process.Dispose()
 $deadline=[DateTime]::UtcNow.AddSeconds(90)
 while (-not (Test-Path (Join-Path $root '{run}-emacs.json'))) {{
   if ([DateTime]::UtcNow -gt $deadline) {{throw 'No Emacs timing result'}}
   Start-Sleep -Milliseconds 100
 }}
 @{{epoch=$start/1000.0; launcher='same C# executable as desktop'; engine='{engine}'}} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $root '{run}-wrapper.json')
 Get-Content (Join-Path $root '{run}-check.json')
 '''
 r=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-EncodedCommand',base64.b64encode(s.encode('utf-16le')).decode()],capture_output=True,text=True)
 print(run,r.returncode,r.stdout[-100:] if r.returncode==0 else r.stderr[-1600:],flush=True)
 if r.returncode: break
(p/'evox-windows.ps1').write_text(final)
