$ErrorActionPreference='Stop'
$old=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-reload-20261113'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-early-20261113'
foreach ($name in @('client','runtime','probe-config')) { Copy-Item -LiteralPath (Join-Path $old $name) -Destination (Join-Path $root $name) -Recurse }
foreach ($name in @('evox-windows-launcher.exe','evox-windows-launcher.cs','native-check.el','build.ps1','run-native.ps1','evox-windows.ps1','runtime-files.json')) { Copy-Item -LiteralPath (Join-Path $old $name) -Destination (Join-Path $root $name) }
Write-Output 'Copied existing native payload and client to isolated candidate.'
