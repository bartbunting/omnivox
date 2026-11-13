#!/bin/sh
# Launch the qualified Omnivox prototype with the normal Emacsvox profile.
set -eu
umask 077

runner=/home/bart/src/omnivox-lightweight-reload/docs/benchmarks/data/2026-11-11-personal-development-launch/start.py
log_directory=/home/bart/.local/state/emacsvox/lightweight-test-launcher
mkdir -p "$log_directory"
launch_log=$(mktemp "$log_directory/startup.XXXXXXXX.log")
printf 'Launch log: %s\n' "$launch_log"

# Retain the desktop launcher's existing keyboard mapping for interactive use.
batch_check=false
for argument in "$@"; do
    case "$argument" in
        --batch-check|--gui-check|--check-launch) batch_check=true ;;
    esac
done
if [ "$batch_check" = false ] && [ -n "${DISPLAY:-}" ] &&
   [ -r /home/bart/.Xmodmap ] && command -v xmodmap >/dev/null 2>&1; then
    xmodmap /home/bart/.Xmodmap >>"$launch_log" 2>&1 || true
fi
exec /usr/bin/python3 "$runner" "$@" >>"$launch_log" 2>&1
