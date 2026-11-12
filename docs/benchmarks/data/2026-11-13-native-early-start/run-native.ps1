
param([string]$Engine='winrt', [switch]$EarlySpeech)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-early-20261113'
$client=Join-Path $root 'client'
. (Join-Path $client 'utils\emacsvox-windows-common.ps1')
$config=[IO.File]::ReadAllText((Join-Path $client 'native-install.json')) | ConvertFrom-Json
$run=Join-Path $root ('logs\native ' + $Engine + '-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'))
New-Item -ItemType Directory -Path $run | Out-Null
[IO.File]::WriteAllText((Join-Path $run 'native-reload-test'),'isolated native Windows acceptance')
Copy-Item -LiteralPath (Join-Path $root 'probe-config\config.json') -Destination (Join-Path $run 'config.json')
$environment=@{
 HOME=$env:USERPROFILE; EMACSVOX_DIR=$client; TTS_PROGRAM=$config.Omnivox;
 ESPEAK_NG_DATA=$config.EspeakData; OMNIVOX_CONFIG_DIR=$run;
 OMNIVOX_VOICE_ROOT=(Join-Path $run 'voices'); XDG_STATE_HOME=(Join-Path $run 'state');
 OMNIVOX_ENGINE=$Engine; OMNIVOX_AUDIO_OUTPUT='null'; OMNIVOX_EXPERIMENTAL_EARLY_SPEECH=$(if ($EarlySpeech) { '1' } else { $null });
 OMNIVOX_NATIVE_TEST_ROOT=$run; EMACSVOX_PLAY=$null;
 OMNIVOX_RHVOICE_LIBRARY=(Join-Path $env:LOCALAPPDATA 'Emacsvox\RHVoice\1.14.0-wrapper-0.5.0-slt\RHVoice.dll');
 OMNIVOX_RHVOICE_DATA=(Join-Path $env:LOCALAPPDATA 'Emacsvox\RHVoice\1.14.0-wrapper-0.5.0-slt\data')
}
Write-Output "Run: $run"
Invoke-EmacsvoxNative $config.Emacs @('-Q','--batch','--eval',(Get-EmacsvoxLoadExpression (Join-Path $root 'run-native-windows-reload.el'))) $environment (Join-Path $run 'emacs.log') 300
Get-Content -LiteralPath (Join-Path $run 'result.json') -Raw
