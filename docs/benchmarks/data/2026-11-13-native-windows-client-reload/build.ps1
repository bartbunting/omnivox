
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-reload-20261113'
$client=Join-Path $root 'client'
. (Join-Path $client 'utils\emacsvox-windows-common.ps1')
$config=[IO.File]::ReadAllText((Join-Path $client 'native-install.json')) | ConvertFrom-Json
$environment=@{ EMACSVOX_NATIVE_ROOT=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($client)); EMACSVOX_NATIVE_BYTECODE='build' }
$arguments=@('-Q','--batch','--eval',(Get-EmacsvoxLoadExpression (Join-Path $client 'utils\emacsvox-native-bytecode.el')),'--funcall','emacsvox-native-bytecode-main')
Invoke-EmacsvoxNative $config.Emacs $arguments $environment (Join-Path $root 'logs\bytecode-build.log') 900
$environment.EMACSVOX_NATIVE_BYTECODE='check'
Invoke-EmacsvoxNative $config.Emacs $arguments $environment (Join-Path $root 'logs\bytecode-check.log') 120
