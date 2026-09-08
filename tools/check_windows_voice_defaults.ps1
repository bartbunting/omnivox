# Silent integration audit; requires installed x86 runtimes and matching built helpers.
param(
    [Parameter(Mandatory = $true)][string]$Helpers,
    [Parameter(Mandatory = $true)][string]$EciDll,
    [Parameter(Mandatory = $true)][string]$DectalkDll
)
if ([IntPtr]::Size -ne 4 -or [Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw 'Run in x86 Windows PowerShell with -Sta.'
}
$ErrorActionPreference = 'Stop'
foreach ($inputFile in @((Join-Path $Helpers 'OmnivoxEloquenceHelper32.exe'), (Join-Path $Helpers 'OmnivoxDectalkHelper32.exe'), $EciDll, $DectalkDll)) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $inputFile).Hash.ToLowerInvariant()
    Write-Output ('input_sha256 ' + (Split-Path -Leaf $inputFile) + ' ' + $hash)
}
Add-Type -Path (Join-Path $PSScriptRoot 'WindowsVoiceDefaultsAudit.cs')
[NativeDefaultsAudit]::Run((Join-Path $Helpers 'OmnivoxEloquenceHelper32.exe'), $EciDll, $true)
[NativeDefaultsAudit]::Run((Join-Path $Helpers 'OmnivoxDectalkHelper32.exe'), $DectalkDll, $false)
Write-Output 'PASS: native set/default/set values restored for every advertised Windows voice.'
