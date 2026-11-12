$ErrorActionPreference = 'Stop'
$root = Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-early-20261113'
$prior = Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-reload-20261113'
$desktop = [Environment]::GetFolderPath('Desktop')
$shortcutPath = Join-Path $desktop 'Emacsvox Windows.lnk'
$target = Join-Path $root 'evox-windows-launcher.exe'
$runtime = Join-Path $root 'runtime\omnivox.exe'
foreach ($manifest in @('runtime-files.json', 'client-files.json')) {
    $base = if ($manifest -eq 'runtime-files.json') { Join-Path $root 'runtime' } else { $root }
    foreach ($file in ([IO.File]::ReadAllText((Join-Path $root $manifest)) | ConvertFrom-Json)) {
        if ((Get-FileHash -LiteralPath (Join-Path $base $file.path)).Hash -ne $file.sha256) {
            throw "Changed payload: $($file.path)"
        }
    }
}
foreach ($engine in @('winrt', 'default', 'dectalk')) {
    $expectedEngine = if ($engine -eq 'default') { 'eloquence' } else { $engine }
    $report = [IO.File]::ReadAllText((Join-Path $root "logs\native-personal-$engine.json")) | ConvertFrom-Json
    if ($report.result -ne 'passed' -or $report.engines.Count -ne 2 -or
        @($report.engines | Where-Object { $_ -ne $expectedEngine }).Count -ne 0 -or
        $report.client_process_cleanup -ne 'retired') { throw "Personal check failed: $engine" }
}
$testProcesses = @(Get-CimInstance Win32_Process -Filter "Name = 'omnivox.exe'" |
    Where-Object { $_.ExecutablePath -eq $runtime })
if ($testProcesses.Count -ne 0) { throw 'Candidate test processes remain; inspect cleanup before deployment.' }
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
if ($shortcut.TargetPath -ne (Join-Path $prior 'evox-windows-launcher.exe')) {
    throw 'Shortcut target changed since inspection; refusing to overwrite it.'
}
$previous = @{ target = $shortcut.TargetPath; working_directory = $shortcut.WorkingDirectory;
    arguments = $shortcut.Arguments; icon = $shortcut.IconLocation; description = $shortcut.Description }
$backups = Join-Path $root 'backups'
New-Item -ItemType Directory -Path $backups -Force | Out-Null
$backup = Join-Path $backups ('Emacsvox Windows.before-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '.lnk')
Copy-Item -LiteralPath $shortcutPath -Destination $backup
$shortcut.TargetPath = $target
$shortcut.WorkingDirectory = $root
$shortcut.Description = 'Native Windows Emacsvox development build with early startup, preference reload and guarded recovery'
$shortcut.Save()
$readback = $shell.CreateShortcut($shortcutPath)
if ($readback.TargetPath -ne $target -or $readback.WorkingDirectory -ne $root -or
    $readback.Arguments -ne $previous.arguments -or $readback.IconLocation -ne $previous.icon) {
    throw 'Shortcut readback did not match the requested update.'
}
$source = [IO.File]::ReadAllText((Join-Path $root 'provenance.json')) | ConvertFrom-Json
$deployment = [ordered]@{
    deployed_utc = [DateTime]::UtcNow.ToString('o'); shortcut = $shortcutPath;
    target = $readback.TargetPath; working_directory = $readback.WorkingDirectory;
    backup = $backup; previous = $previous; source = $source;
    launcher_sha256 = (Get-FileHash -LiteralPath $target).Hash.ToLower();
    startup_sha256 = (Get-FileHash -LiteralPath (Join-Path $root 'evox-windows.ps1')).Hash.ToLower();
    early_speech = $true; default_engine = 'eloquence'; runtime_processes_after_tests = $testProcesses.Count;
    personal_settings = 'Preserved existing 0c72147cc engine-config and voices';
    running_sessions = 'Unchanged; fresh shortcut launch loads new client'
}
$json = $deployment | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $root 'deployment.json'), $json, (New-Object Text.UTF8Encoding($false)))
Write-Output $json
