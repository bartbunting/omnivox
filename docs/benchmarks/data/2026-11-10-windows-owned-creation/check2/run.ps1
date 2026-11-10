$ErrorActionPreference = 'Stop'
$env:OMNIVOX_TEST_GUARD_EXECUTABLE = 'C:\Users\bart\AppData\Local\Temp\omnivox-lightweight-20261110\Emacsvox\Omnivox\runtime\1c1833281316787d\omnivox.exe'
$env:OMNIVOX_TEST_GUARD_HELPER = 'C:\Users\bart\AppData\Local\Temp\omnivox-lightweight-20261110\Emacsvox\Omnivox\runtime\1c1833281316787d\flite\omnivox-flite-helper.exe'
& "$PSScriptRoot\tests.exe" native_guard::windows_owned::tests --include-ignored --nocapture --test-threads=1 *> "$PSScriptRoot\owned.log"
$owned = $LASTEXITCODE
& "$PSScriptRoot\tests.exe" native_guard::os::native::creation --nocapture *> "$PSScriptRoot\creation.log"
$creation = $LASTEXITCODE
@{owned=$owned;creation=$creation} | ConvertTo-Json | Set-Content "$PSScriptRoot\exit.json"
if ($owned -ne 0 -or $creation -ne 0) { exit 1 }
