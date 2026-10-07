#!/usr/bin/env bash
# Stop the vpm_e2e emulator, the tone ffmpeg and the vpm_e2e_src null sink.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
ADB="$SDK/platform-tools/adb"
STATE="$REPO/.cache/e2e"
SINK=vpm_e2e_src

serial=$(cat "$STATE/serial" 2>/dev/null || true)
[ -n "$serial" ] && "$ADB" -s "$serial" emu kill >/dev/null 2>&1
for _ in $(seq 1 20); do pgrep -f "^[^ ]*qemu-system-x86_64 .*-avd vpm_e2e" >/dev/null || break; sleep 1; done
pkill -f "^[^ ]*qemu-system-x86_64 .*-avd vpm_e2e" 2>/dev/null && sleep 2
pkill -9 -f "^[^ ]*qemu-system-x86_64 .*-avd vpm_e2e" 2>/dev/null

pkill -f "^ffmpeg .*-device $SINK" 2>/dev/null
# unload every null-sink module named vpm_e2e_src (covers ones not created by start script)
pactl list short modules | awk -v s="sink_name=$SINK" '$2=="module-null-sink" && index($0,s){print $1}' |
  while read -r m; do pactl unload-module "$m"; done
rm -f "$STATE/serial" "$STATE/sink_module"
echo "[stop_emulator] stopped"
