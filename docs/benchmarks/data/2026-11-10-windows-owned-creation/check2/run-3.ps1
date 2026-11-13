$ErrorActionPreference = 'Stop'
$env:OMNIVOX_TEST_GUARD_EXECUTABLE = 'C:\Users\bart\AppData\Local\Temp\omnivox-lightweight-20261110\Emacsvox\Omnivox\runtime\1c1833281316787d\omnivox.exe'
$env:OMNIVOX_TEST_GUARD_HELPER = 'C:\Users\bart\AppData\Local\Temp\omnivox-lightweight-20261110\Emacsvox\Omnivox\runtime\1c1833281316787d\flite\omnivox-flite-helper.exe'

$root = Join-Path $PSScriptRoot 'run-3'
New-Item -ItemType Directory $root | Out-Null
$p = Start-Process -FilePath "$PSScriptRoot\tests.exe" -ArgumentList @('native_guard::windows_owned::tests','--include-ignored','--nocapture','--test-threads=1') -PassThru -Wait -RedirectStandardOutput "$root\owned.stdout" -RedirectStandardError "$root\owned.stderr"
$owned=$p.ExitCode
$p = Start-Process -FilePath "$PSScriptRoot\tests.exe" -ArgumentList @('native_guard::os::native::creation','--nocapture') -PassThru -Wait -RedirectStandardOutput "$root\creation.stdout" -RedirectStandardError "$root\creation.stderr"
$creation=$p.ExitCode
@{owned=$owned;creation=$creation} | ConvertTo-Json | Set-Content "$root\exit.json"
if ($owned -ne 0 -or $creation -ne 0) {exit 1}
