$pinfo = New-Object System.Diagnostics.ProcessStartInfo
$omnivox = if ($env:OMNIVOX_BIN) { $env:OMNIVOX_BIN } else { ".\target\release\omnivox.exe" }
$pinfo.FileName = (Get-Command $omnivox -ErrorAction Stop).Source
$pinfo.RedirectStandardInput = $true
$pinfo.UseShellExecute = $false

$p = [System.Diagnostics.Process]::Start($pinfo)

# Test tones
$p.StandardInput.WriteLine("t 500 100")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 300

$p.StandardInput.WriteLine("t 800 100")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 300

$p.StandardInput.WriteLine("t 1000 200")
$p.StandardInput.WriteLine("d")
Start-Sleep -Milliseconds 500

# Speech after tones
$p.StandardInput.WriteLine("tts_say {Tones complete. Now testing speech alongside a tone.}")
Start-Sleep -Milliseconds 200
$p.StandardInput.WriteLine("t 440 500")
$p.StandardInput.WriteLine("d")

$p.StandardInput.Flush()
Start-Sleep -Seconds 5

$p.StandardInput.Close()
$p.WaitForExit(3000)
if (-not $p.HasExited) { $p.Kill() }

Write-Host "Done."
