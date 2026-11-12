from pathlib import Path
import shutil
stage=Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/NativeLightweight/native-early-20261113')
p=stage/'startup-timing-20261113'
source=Path('/home/bart/src/workstation-config/windows/emacs')
s=(stage/'evox-windows.ps1').read_text()
s=s.replace('$selected = Get-EmacsvoxNativeEmacs $config.Emacs', '''$command = Get-Command $config.Emacs -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $command) { throw "Selected Emacs does not exist: $($config.Emacs)" }
$selected = @{ Program = $command.Source }''')
a=s.index('Invoke-EmacsvoxNative $selected.Program @(\'-Q\', \'--batch\'')
b=s.index('$arguments = @',a)
block=s[a:b]
s=s[:a]+'''# Deployment and explicit checks validate the complete build. Ordinary startup
# opens only the personal Emacs; its bootstrap checks the running Emacs version.
if ($Check) {
    $selected = Get-EmacsvoxNativeEmacs $config.Emacs
'''+''.join('    '+line+'\n' for line in block.splitlines())+'}\n'+s[b:]
(p/'final-launcher.ps1').write_text(s)
shutil.copy2(source/'evox-windows-init.el',p/'final-bootstrap.el')
# Explicit check on the exact final script plus isolated result/log names.
s=s.replace("$startup = Join-Path $env:USERPROFILE 'bin\\evox-windows-init.el'", "$startup = Join-Path $PSScriptRoot 'final-bootstrap.el'")
s=s.replace("$result = Join-Path $development 'logs\\native-guarded-personal-check.json'", "$result = Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-check.json')")
s=s.replace("(Join-Path $development 'logs\\native-guarded-personal-check.log')", "(Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-check.log'))")
(p/'validate-final.ps1').write_text(s)
# Wrapper-equivalent normal launch timing. Same C# executable, changed only the
# profile expression and cleanup fixture after startup in the measurement copy.
s=(p/'final-launcher.ps1').read_text()
s=s.replace('Set-StrictMode -Version Latest', '''[IO.File]::WriteAllText((Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-script-start.txt')), [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds().ToString())
Set-StrictMode -Version Latest''')
s=s.replace('(Get-EmacsvoxLoadExpression $startup))', "(Get-EmacsvoxLoadExpression (Join-Path $PSScriptRoot 'trace.el')))")
s=s.replace('if ($Check) {\n    $result', '''$environment.EVOX_TIMING_ROOT = $PSScriptRoot
$environment.EMACSVOX_NATIVE_PERSONAL_CHECK = '1'
$environment.EMACSVOX_NATIVE_PROBE_RESULT = Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-check.json')
$environment.XDG_STATE_HOME = Join-Path $PSScriptRoot ($env:EVOX_TIMING_RUN + '-state')
$arguments += @('--eval', (Get-EmacsvoxLoadExpression (Join-Path $PSScriptRoot 'finish.el')))
if ($Check) {
    $result''')
(p/'evox-windows.ps1').write_text(s)
shutil.copy2(stage/'evox-windows-launcher.exe',p/'evox-windows-launcher.exe')
