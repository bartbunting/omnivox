# Copyright (C) 2026 Bart Bunting
# SPDX-License-Identifier: GPL-2.0-or-later
# Run each engine in a separate bounded x86 STA process. Captures PCM silently.
param(
    [Parameter(Mandatory = $true)][ValidateSet('eloquence', 'dectalk')][string]$Engine,
    [Parameter(Mandatory = $true)][string]$Helper,
    [Parameter(Mandatory = $true)][string]$RuntimeDll
)
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 4 -or [Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw 'Run in x86 Windows PowerShell with -Sta.'
}
foreach ($inputFile in @($Helper, $RuntimeDll)) {
    if (!(Test-Path -LiteralPath $inputFile -PathType Leaf)) {
        throw "Missing audit input: $inputFile"
    }
}
Add-Type -Path (Join-Path $PSScriptRoot 'NativeVoiceParametersAudit.cs')
$result = [NativeVoiceParametersAudit]::Run($Helper, $RuntimeDll, $Engine -eq 'eloquence')
$result['helper_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $Helper).Hash.ToLowerInvariant()
$result['runtime_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $RuntimeDll).Hash.ToLowerInvariant()
$result['audit_source_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $PSScriptRoot 'NativeVoiceParametersAudit.cs')).Hash.ToLowerInvariant()
$result | ConvertTo-Json -Depth 12
if (!$result['all_candidates_passed']) {
    throw "Native parameter audit found $($result['failed_cases']) unqualified cases. See JSON results."
}
