param(
    [switch]$Clean,
    [ValidateSet("all", "eloquence", "dectalk")]
    [string]$Engine = "all",
    [string]$OutputDirectory = "bin",
    [string]$CompilerPath,
    [string]$ReferenceDirectory
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Common = Join-Path $Root "common"
$Bin = Join-Path $Root $OutputDirectory

if ($Clean) {
    if (Test-Path $Bin) {
        Remove-Item -Recurse -Force $Bin
    }
    exit 0
}

$CompilerArguments = @("/nologo")
if (![string]::IsNullOrEmpty($CompilerPath)) {
    if ([string]::IsNullOrEmpty($ReferenceDirectory)) {
        throw "ReferenceDirectory is required with CompilerPath"
    }
    $Compiler = $CompilerPath
    $CompilerArguments += @(
        "/deterministic+",
        "/debug-",
        "/nostdlib+",
        "/reference:$ReferenceDirectory\mscorlib.dll",
        "/reference:$ReferenceDirectory\System.dll",
        "/reference:$ReferenceDirectory\System.Core.dll",
        "/reference:$ReferenceDirectory\System.Web.Extensions.dll"
    )
} else {
    $Compiler = Join-Path $env:WINDIR `
        "Microsoft.NET\Framework64\v4.0.30319\csc.exe"
}
if (!(Test-Path $Compiler)) {
    throw "The C# compiler was not found at $Compiler"
}

New-Item -ItemType Directory -Force $Bin | Out-Null

if ($Engine -eq "all" -or $Engine -eq "eloquence") {
    & $Compiler @CompilerArguments /target:exe /optimize+ /platform:x86 `
        "/out:$Bin\OmnivoxEloquenceHelper32.exe" `
        (Join-Path $Root "eloquence\OmnivoxEloquenceCapture.cs") `
        (Join-Path $Root "eloquence\OmnivoxEloquenceHelper.cs") `
        (Join-Path $Common "OmnivoxNativeLibrary.cs") `
        (Join-Path $Common "OmnivoxHelperHost.cs")
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to build OmnivoxEloquenceHelper32.exe"
    }
}

if ($Engine -eq "all" -or $Engine -eq "dectalk") {
    & $Compiler @CompilerArguments /target:exe /optimize+ /platform:x86 `
        "/out:$Bin\OmnivoxDectalkHelper32.exe" `
        (Join-Path $Root "dectalk\OmnivoxDectalkCapture.cs") `
        (Join-Path $Root "dectalk\OmnivoxDectalkHelper.cs") `
        (Join-Path $Common "OmnivoxNativeLibrary.cs") `
        (Join-Path $Common "OmnivoxHelperHost.cs")
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to build OmnivoxDectalkHelper32.exe"
    }
}

Write-Output "Built Omnivox Windows helpers under $Bin"
