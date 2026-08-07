<#
.SYNOPSIS
Enables or disables bounded Windows Error Reporting dumps for Omnivox.

.DESCRIPTION
This script must run from an elevated PowerShell. Full dumps can contain
spoken text, voice settings, paths, and unrelated process memory. Keep them
private and disable collection after the failure has been captured.
#>

[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [switch]$Disable,
    [switch]$IncludeServer,
    [ValidateRange(1, 20)]
    [int]$DumpCount = 5,
    [string]$DumpFolder =
        (Join-Path $env:LOCALAPPDATA "Emacsvox\Omnivox\dumps")
)

$ErrorActionPreference = "Stop"
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$administrator = [Security.Principal.WindowsBuiltInRole]::Administrator
if (!$principal.IsInRole($administrator)) {
    throw "Run this script from an elevated PowerShell."
}

$nativeRoot =
    "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps"
$wowRoot =
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\Windows Error Reporting\LocalDumps"
$targets = @(
    [pscustomobject]@{
        Name = "OmnivoxEloquenceHelper32.exe"
        Roots = @($nativeRoot, $wowRoot)
    },
    [pscustomobject]@{
        Name = "OmnivoxDectalkHelper32.exe"
        Roots = @($nativeRoot, $wowRoot)
    }
)
if ($IncludeServer) {
    $targets += [pscustomobject]@{
        Name = "omnivox.exe"
        Roots = @($nativeRoot)
    }
}

foreach ($target in $targets) {
    foreach ($root in $target.Roots) {
        $key = Join-Path $root $target.Name
        if ($Disable) {
            if ((Test-Path -LiteralPath $key) -and
                $PSCmdlet.ShouldProcess($key, "Remove crash-dump policy")) {
                Remove-Item -LiteralPath $key -Recurse -Force
            }
            continue
        }

        if ($PSCmdlet.ShouldProcess($key, "Configure full crash dumps")) {
            New-Item -ItemType Directory -Force -Path $DumpFolder |
                Out-Null
            New-Item -Path $root -Force | Out-Null
            New-Item -Path $key -Force | Out-Null
            New-ItemProperty -LiteralPath $key -Name DumpFolder -PropertyType ExpandString -Value $DumpFolder -Force |
                Out-Null
            New-ItemProperty -LiteralPath $key -Name DumpCount -PropertyType DWord -Value $DumpCount -Force |
                Out-Null
            New-ItemProperty -LiteralPath $key -Name DumpType -PropertyType DWord -Value 2 -Force |
                Out-Null
        }
    }
}

if ($Disable) {
    Write-Output "Omnivox crash-dump policy removed."
} else {
    Write-Warning "Full dumps may contain spoken text and other private memory."
    Write-Output "Omnivox crash dumps will be written to $DumpFolder"
}
