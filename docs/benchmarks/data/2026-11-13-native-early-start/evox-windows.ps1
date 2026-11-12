[CmdletBinding()]
param(
    [switch]$Check,
    [string]$NativeCheckout = (Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-early-20261113\client')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $NativeCheckout 'utils\emacsvox-windows-common.ps1')
Assert-NativeWindows
Assert-WindowsLocalPath $NativeCheckout
$config = [IO.File]::ReadAllText((Join-Path $NativeCheckout 'native-install.json')) | ConvertFrom-Json
if ($config.Schema -ne 1 -or $config.Role -ne 'Desktop') { throw 'Unsupported native installation configuration.' }
$selected = Get-EmacsvoxNativeEmacs $config.Emacs
if (-not (Test-Path -LiteralPath $config.Omnivox)) { throw 'Installed Omnivox is missing.' }
$personal = Join-Path $env:LOCALAPPDATA 'Emacsvox\Personal'
$agentTools = @((Join-Path $personal 'tools\node-v24.20.0-win-x64'),
    (Join-Path $personal 'tools\codex-acp'))
$startup = Join-Path $env:USERPROFILE 'bin\evox-windows-init.el'
$development = Split-Path $NativeCheckout -Parent
foreach ($file in @($startup, (Join-Path $personal 'config\early-init.el'),
        (Join-Path $personal 'config\init.el'), (Join-Path $personal 'config\emacsvox-config.org'))) {
    if (-not (Test-Path -LiteralPath $file)) { throw "Missing personal configuration: $file" }
}
$environment = @{
    PATH = (($agentTools + @($env:PATH)) -join ';');
    HOME = $env:USERPROFILE; EMACS = $null; EMACSVOX_DIR = $NativeCheckout;
    EMACSVOX_NATIVE_ROOT = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($NativeCheckout));
    EMACSVOX_NATIVE_BYTECODE = 'check'; TTS_PROGRAM = $config.Omnivox;
    ESPEAK_NG_DATA = $config.EspeakData;
    OMNIVOX_CONFIG_DIR = $config.EngineConfig;
    OMNIVOX_VOICE_ROOT = $config.VoiceRoot;
    OMNIVOX_EXPERIMENTAL_EARLY_SPEECH = '1';
    OMNIVOX_ENGINE = 'eloquence'; EMACSVOX_PLAY = $null; OMNIVOX_AUDIO_OUTPUT = $null;
    EMACSVOX_PERSONAL_DIRECTORY = $personal;
    EMACSVOX_PACKAGE_DIRECTORY = (Join-Path $personal 'elpa');
    EMACSVOX_CONFIG_FILE = (Join-Path $personal 'config\emacsvox-config.org');
    XDG_STATE_HOME = (Join-Path $personal 'state'); BB_EVOX_CHECK = $null
}
if ($env:OMNIVOX_ENGINE) { $environment.OMNIVOX_ENGINE = $env:OMNIVOX_ENGINE }
Invoke-EmacsvoxNative $selected.Program @('-Q', '--batch', '--eval',
    (Get-EmacsvoxLoadExpression (Join-Path $NativeCheckout 'utils\emacsvox-native-bytecode.el')),
    '--funcall', 'emacsvox-native-bytecode-main') $environment | Out-Null
$arguments = @('-Q', '--no-splash', '--name', 'evox-windows',
    '--eval', '(setq omnivox-native-guard--enabled t)', '--eval',
    (Get-EmacsvoxLoadExpression $startup))
if ($Check) {
    $result = Join-Path $development 'logs\native-guarded-personal-check.json'
    $environment.EMACSVOX_NATIVE_PERSONAL_CHECK = '1'
    $environment.EMACSVOX_NATIVE_PROBE_RESULT = $result
    $arguments += @('--eval', (Get-EmacsvoxLoadExpression (Join-Path $development 'native-check.el')))
    Invoke-EmacsvoxNative $selected.Program $arguments $environment (Join-Path $development 'logs\native-guarded-personal-check.log') 180 | Out-Null
    if (-not (Test-Path $result)) { throw "Missing native personal check result: $result" }
    $report = [IO.File]::ReadAllText($result) | ConvertFrom-Json
    if ($report.result -ne 'passed') { throw "Native personal check failed: $($report.error)" }
    Write-Host ([IO.File]::ReadAllText($result))
} else {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $selected.Program
    $start.Arguments = ($arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' '
    $start.WorkingDirectory = $env:USERPROFILE
    $start.UseShellExecute = $false
    foreach ($key in $environment.Keys) {
        if ($null -eq $environment[$key]) { $start.EnvironmentVariables.Remove($key) }
        else { $start.EnvironmentVariables[$key] = [string]$environment[$key] }
    }
    $process = [Diagnostics.Process]::Start($start)
    Write-Host "Started personal native Emacsvox (PID $($process.Id))."
    $process.Dispose()
}
