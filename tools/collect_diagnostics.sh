#!/usr/bin/env bash
# Collect privacy-conscious Omnivox failure evidence from WSL and Windows.

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
omnivox_directory=$(CDPATH= cd -- "$script_directory/.." && pwd)
emacsvox_directory=${EMACSVOX_SOURCE_DIRECTORY:-"$(dirname -- "$omnivox_directory")/emacsvox"}
state_directory=${XDG_STATE_HOME:-"$HOME/.local/state"}
log_directory=${OMNIVOX_LOG_DIRECTORY:-"$state_directory/emacsvox/omnivox"}
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
output=${1:-"/tmp/omnivox-diagnostics-$timestamp.tar.gz"}
bundle_directory=$(mktemp -d "${TMPDIR:-/tmp}/omnivox-diagnostics.XXXXXX")
redactor="$script_directory/redact_diagnostics.py"

redact_stream() {
    python3 "$redactor" \
        --private "$HOME" \
        --private "$omnivox_directory" \
        --private "$emacsvox_directory" \
        --private "$log_directory"
}

cleanup() {
    rm -rf -- "$bundle_directory"
}
trap cleanup EXIT HUP INT TERM

mkdir -p -- "$bundle_directory/logs"

{
    printf 'collected_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'omnivox_checkout_present=%s\n' "$([ -d "$omnivox_directory/.git" ] && printf yes || printf no)"
    printf 'emacsvox_checkout_present=%s\n' "$([ -d "$emacsvox_directory/.git" ] && printf yes || printf no)"
    printf 'log_directory_present=%s\n' "$([ -d "$log_directory" ] && printf yes || printf no)"
    uname -srmo
    rustc --version 2>&1 || true
    cargo --version 2>&1 || true
    git -C "$omnivox_directory" rev-parse HEAD 2>&1 || true
    printf 'omnivox_worktree_changes='
    git -C "$omnivox_directory" status --porcelain 2>/dev/null | wc -l || true
    if [ -d "$emacsvox_directory/.git" ]; then
        git -C "$emacsvox_directory" rev-parse HEAD 2>&1 || true
        printf 'emacsvox_worktree_changes='
        git -C "$emacsvox_directory" status --porcelain 2>/dev/null | wc -l || true
    fi
} >"$bundle_directory/overview.txt"

# Retain startup context and the failure tail while bounding archive size.
if [ -d "$log_directory" ]; then
    find "$log_directory" -maxdepth 1 -type f -name 'omnivox-*.log' \
        -mmin -1440 -print0 |
    while IFS= read -r -d '' log_file; do
        log_name=$(basename -- "$log_file")
        {
            sed -n '1,200p' "$log_file"
            printf '\n--- final 20000 lines ---\n'
            tail -n 20000 "$log_file"
        } | redact_stream >"$bundle_directory/logs/$log_name"
    done
fi

ps -eo pid=,ppid=,comm= 2>&1 |
    awk '$3 ~ /^(omnivox|Omnivox)/ { print }' \
    >"$bundle_directory/wsl-processes.txt" || true

if command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -NoProfile -Command '
        Get-CimInstance Win32_Process |
            Where-Object {
                $_.Name -in @(
                    "omnivox.exe",
                    "OmnivoxEloquenceHelper32.exe",
                    "OmnivoxDectalkHelper32.exe"
                )
            } |
            Select-Object ProcessId, ParentProcessId, Name, CreationDate |
            Sort-Object CreationDate |
            Format-List
    ' 2>&1 | redact_stream >"$bundle_directory/windows-processes.txt" || true

    powershell.exe -NoProfile -Command '
        Get-WinEvent -FilterHashtable @{
            LogName = "Application"
            StartTime = (Get-Date).AddDays(-1)
        } -ErrorAction SilentlyContinue |
            Where-Object {
                $_.ProviderName -match "Application Error|Windows Error Reporting|.NET Runtime" -or
                $_.Message -match "Omnivox|Eloquence|DECtalk|ECI.DLL"
            } |
            Select-Object TimeCreated, Id, ProviderName, LevelDisplayName,
                Message |
            Format-List
    ' 2>&1 | redact_stream >"$bundle_directory/windows-events.txt" || true

    powershell.exe -NoProfile -Command '
        $dumpDirectory = Join-Path $env:LOCALAPPDATA "Emacsvox\Omnivox\dumps"
        "dump_directory_configured=$($null -ne $dumpDirectory)"
        Get-ChildItem -LiteralPath $dumpDirectory -Filter *.dmp -ErrorAction SilentlyContinue |
            Select-Object Name, Length, CreationTimeUtc, LastWriteTimeUtc |
            Format-List
        $roots = @(
            "HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps",
            "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\Windows Error Reporting\LocalDumps"
        )
        foreach ($root in $roots) {
            Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue |
                Where-Object { $_.PSChildName -match "^Omnivox" } |
                ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath }
        }
    ' 2>&1 | redact_stream >"$bundle_directory/windows-dumps.txt" || true
fi

if [ -d "$emacsvox_directory/servers/omnivox-bin/current" ]; then
    resolved_runtime=$(readlink -f -- "$emacsvox_directory/servers/omnivox-bin/current")
    printf 'runtime_present=yes\n' >"$bundle_directory/runtime.txt"
    find "$resolved_runtime" -maxdepth 1 -type f \
        \( -iname '*.exe' -o -iname '*.dll' \) -print0 |
    while IFS= read -r -d '' runtime_file; do
        runtime_hash=$(sha256sum "$runtime_file" | cut -d ' ' -f 1)
        printf '%s  %s\n' "$runtime_hash" "$(basename -- "$runtime_file")"
    done >>"$bundle_directory/runtime.txt"
fi

mkdir -p -- "$(dirname -- "$output")"
(umask 077 && tar -czf "$output" -C "$bundle_directory" .)
chmod 600 -- "$output"
printf '%s\n' "$output"
