# Copyright (C) 2026 Bart Bunting
# SPDX-License-Identifier: GPL-2.0-or-later
param(
    [Parameter(Mandatory = $true)][string]$Helper,
    [string]$RuntimeDll,
    [switch]$PlanningOnly
)
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 4 -or [Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw 'Run in x86 Windows PowerShell with -Sta.'
}
if (!$PlanningOnly -and !(Test-Path -LiteralPath $RuntimeDll -PathType Leaf)) {
    throw 'RuntimeDll must name the user-installed ECI DLL.'
}
$Source = Join-Path $PSScriptRoot 'EloquenceExecutionAudit.cs'
$Fixtures = Join-Path (Split-Path -Parent $PSScriptRoot) 'docs\protocol-fixtures\engine-voice-parameters.json'
Add-Type -Path $Source -ReferencedAssemblies System.Web.Extensions.dll
$result = @{ planning = [EloquenceExecutionAudit]::Planning($Helper, $Fixtures) }
if (!$PlanningOnly) {
    $result['runtime'] = [EloquenceExecutionAudit]::Runtime($Helper, $RuntimeDll, $Fixtures)
    $result['runtime_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $RuntimeDll).Hash.ToLowerInvariant()
}
$result['helper_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $Helper).Hash.ToLowerInvariant()
$result['probe_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $Source).Hash.ToLowerInvariant()
$result | ConvertTo-Json -Depth 12
