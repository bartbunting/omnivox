# Copyright (C) 2026 Bart Bunting
# SPDX-License-Identifier: GPL-2.0-or-later
# Run against the shared host without installing or loading a native speech DLL.
param([string]$HostSource)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
if (!$HostSource) {
    $HostSource = Join-Path $Root 'windows-helpers\common\OmnivoxHelperHost.cs'
}
$Compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$Scratch = Join-Path ([IO.Path]::GetTempPath()) ('omnivox-cancel-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $Scratch | Out-Null
try {
    $Executable = Join-Path $Scratch 'CancellationTests.exe'
    & $Compiler /nologo /target:exe /platform:x86 /reference:System.Web.Extensions.dll `
        "/out:$Executable" (Join-Path $PSScriptRoot 'WindowsHelperCancellationTests.cs') `
        $HostSource (Join-Path $Root 'windows-helpers\common\OmnivoxNativeLibrary.cs') `
        (Join-Path $Root 'windows-helpers\common\OmnivoxHelperParameters.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Cancellation test build failed' }
    $Process = New-Object System.Diagnostics.Process
    $Process.StartInfo.FileName = $Executable
    $Process.StartInfo.UseShellExecute = $false
    $Process.Start() | Out-Null
    if (!$Process.WaitForExit(60000)) {
        $Process.Kill()
        $Process.WaitForExit()
        throw 'Cancellation tests exceeded 60 seconds'
    }
    if ($Process.ExitCode -ne 0) { throw "Cancellation tests failed: $($Process.ExitCode)" }
} finally {
    if ($Process) { $Process.Dispose() }
    Remove-Item -LiteralPath $Scratch -Recurse -Force
}
