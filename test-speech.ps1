$pinfo = New-Object System.Diagnostics.ProcessStartInfo
$pinfo.FileName = ".\target\release\omnivox.exe"
$pinfo.RedirectStandardInput = $true
$pinfo.UseShellExecute = $false

$p = [System.Diagnostics.Process]::Start($pinfo)
$p.StandardInput.WriteLine("tts_say {Hello! This is Omnivox running on Windows with the modern WinRT speech engine.}")
$p.StandardInput.Flush()

Start-Sleep -Seconds 6

$p.StandardInput.Close()
$p.WaitForExit(3000)
if (-not $p.HasExited) { $p.Kill() }

Write-Host "Done."
