param([string]$Helper, [string]$RuntimeDll, [switch]$CompileOnly)
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 4 -or [Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') { throw 'Use x86 PowerShell -Sta.' }
$source = Join-Path $PSScriptRoot 'DectalkTimingAudit.cs'
Add-Type -Path $source
if ($CompileOnly) { Write-Output 'Timing probe compiled'; exit 0 }
$result = [DectalkTimingAudit]::Run($Helper, $RuntimeDll, 3, 30)
$result['helper_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $Helper).Hash.ToLowerInvariant()
$result['dll_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $RuntimeDll).Hash.ToLowerInvariant()
$result['probe_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
$result | ConvertTo-Json -Depth 15
