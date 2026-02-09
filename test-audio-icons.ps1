$pinfo = New-Object System.Diagnostics.ProcessStartInfo
$pinfo.FileName = ".\target\release\omnivox.exe"
$pinfo.RedirectStandardInput = $true
$pinfo.UseShellExecute = $false

$p = [System.Diagnostics.Process]::Start($pinfo)

# Get the script directory for absolute paths
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$soundDir = Join-Path $scriptDir "test-sounds"

# Test audio icons via queue (a command) + dispatch (d command)
$p.StandardInput.WriteLine("a $soundDir\button.ogg")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 800

$p.StandardInput.WriteLine("a $soundDir\complete.ogg")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 800

$p.StandardInput.WriteLine("a $soundDir\alarm.ogg")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 800

# Test play sound (p command) - plays immediately without dispatch
$p.StandardInput.WriteLine("tts_say {Now testing play sound command.}")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 200
$p.StandardInput.WriteLine("p $soundDir\button.ogg")
Start-Sleep -Milliseconds 800

# Test concurrent: speech + audio icon
$p.StandardInput.WriteLine("tts_say {Audio icons and speech playing together.}")
$p.StandardInput.WriteLine("a $soundDir\complete.ogg")
$p.StandardInput.WriteLine("d")

$p.StandardInput.Flush()
Start-Sleep -Seconds 5

$p.StandardInput.Close()
$p.WaitForExit(3000)
if (-not $p.HasExited) { $p.Kill() }

Write-Host "Done."
