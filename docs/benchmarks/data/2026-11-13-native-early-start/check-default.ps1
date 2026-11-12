$ErrorActionPreference='Stop'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-reload-20261113'
$previous=$env:OMNIVOX_ENGINE
try {
 Remove-Item Env:OMNIVOX_ENGINE -ErrorAction SilentlyContinue
 & (Join-Path $root 'evox-windows.ps1') -Check
 foreach ($extension in @('json','log')) {
  Copy-Item -LiteralPath (Join-Path $root ("logs\native-guarded-personal-check."+$extension)) -Destination (Join-Path $root ("logs\native-personal-default-eloquence."+$extension))
 }
} finally { $env:OMNIVOX_ENGINE=$previous }
