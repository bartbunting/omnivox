
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$prior=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\0c72147cc'
$root=Join-Path $env:LOCALAPPDATA 'Emacsvox\NativeLightweight\native-reload-20261113'
$runtime=Join-Path $root 'runtime'
if (Test-Path -LiteralPath $runtime) { throw 'Runtime already exists; inspect before changing it.' }
Copy-Item -LiteralPath (Join-Path $prior 'runtime') -Destination $runtime -Recurse
foreach ($entry in Get-ChildItem -LiteralPath (Join-Path $prior 'espeak-ng-data')) {
 Copy-Item -LiteralPath $entry.FullName -Destination (Join-Path $runtime $entry.Name) -Recurse
}
if (-not (Test-Path -LiteralPath (Join-Path $runtime 'espeak-ng-data\phontab'))) { throw 'Incomplete emergency data layout.' }
$files=@(foreach ($pair in @(@{source=(Join-Path $prior 'runtime'); target=$runtime},@{source=(Join-Path $prior 'espeak-ng-data'); target=$runtime})) {
 foreach ($file in Get-ChildItem -LiteralPath $pair.source -File -Recurse) {
  $relative=$file.FullName.Substring($pair.source.Length+1)
  $expected=(Get-FileHash -LiteralPath $file.FullName).Hash
  $actual=(Get-FileHash -LiteralPath (Join-Path $pair.target $relative)).Hash
  if ($expected -ne $actual) { throw "Runtime copy mismatch: $relative" }
  [PSCustomObject]@{path=$relative; sha256=$actual.ToLower()}
 }
})
$files | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'runtime-files.json') -Encoding UTF8
$configPath=Join-Path $root 'client\native-install.json'
$config=[IO.File]::ReadAllText($configPath) | ConvertFrom-Json
$config.Omnivox=Join-Path $runtime 'omnivox.exe'
$config.EspeakData=$runtime
[IO.File]::WriteAllText($configPath,($config | ConvertTo-Json -Depth 5),(New-Object Text.UTF8Encoding($false)))
"Verified $($files.Count) runtime/data files; matching eSpeak data is beside omnivox.exe."
