$pinfo = New-Object System.Diagnostics.ProcessStartInfo
$pinfo.FileName = ".\target\release\omnivox.exe"
$pinfo.RedirectStandardInput = $true
$pinfo.UseShellExecute = $false

$p = [System.Diagnostics.Process]::Start($pinfo)

# Queue speech and tones together - they play concurrently on separate streams
$p.StandardInput.WriteLine("tts_say {This is a longer sentence to test that tones play concurrently during speech synthesis on Windows.}")
$p.StandardInput.WriteLine("t 440 200")
$p.StandardInput.WriteLine("d")

Start-Sleep -Milliseconds 1500

$p.StandardInput.WriteLine("t 660 200")
$p.StandardInput.WriteLine("d")

Start-Sleep -Milliseconds 1500

$p.StandardInput.WriteLine("t 880 200")
$p.StandardInput.WriteLine("d")

$p.StandardInput.Flush()
Start-Sleep -Seconds 5

$p.StandardInput.Close()
$p.WaitForExit(3000)
if (-not $p.HasExited) { $p.Kill() }

Write-Host "Done."
