$ErrorActionPreference='Stop'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-early-20261113'
$previous=$env:OMNIVOX_ENGINE
try {
 foreach ($engine in @('default','dectalk','winrt')) {
  if ($engine -eq 'default') { Remove-Item Env:OMNIVOX_ENGINE -ErrorAction SilentlyContinue }
  else { $env:OMNIVOX_ENGINE=$engine }
  & (Join-Path $root 'evox-windows.ps1') -Check
  foreach ($extension in @('json','log')) {
   Copy-Item -LiteralPath (Join-Path $root ("logs\native-guarded-personal-check."+$extension)) -Destination (Join-Path $root ("logs\native-personal-"+$engine+"."+$extension))
  }
 }
} finally { $env:OMNIVOX_ENGINE=$previous }
