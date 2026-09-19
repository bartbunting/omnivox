param([string]$Helper)
$ErrorActionPreference='Stop'
Add-Type -Path '\\wsl.localhost\Ubuntu-26.04\home\bart\src\omnivox\tools\DectalkExecutionAudit.cs' -ReferencedAssemblies System.Web.Extensions.dll
[DectalkExecutionAudit]::BatchedParameters($Helper, 'C:\Users\bart\AppData\Local\Omnivox\runtimes\dectalk\x86\DECtalk.dll') | ConvertTo-Json -Depth 12
