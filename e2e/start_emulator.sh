#!/usr/bin/env bash
# Start the vpm_e2e AVD fully headless with the host "mic" fed by a 440 Hz tone.
#
# Why the odd flags (see PLAN.md / e2e notes):
#  * TCMALLOC_SKIP_SBRK=true  - the emulator's bundled tcmalloc hands out brk-heap memory;
#    SwiftShader's JIT then mprotect(PROT_EXEC)s it, SELinux denies `execheap`, and the
#    RenderThread jumps into non-executable heap -> SIGSEGV ~60 s into boot. Forcing tcmalloc
#    to mmap avoids the heap entirely (no policy change needed).
#  * -qt-hide-window (not -no-window) - -no-window selects qemu-system-x86_64-headless, which is
#    built WITHOUT the PulseAudio backend ("Could not init `pa' audio driver" -> silent mic).
#    The full binary + hidden window + QT_QPA_PLATFORM=offscreen + no DISPLAY/WAYLAND_DISPLAY
#    is display-independent and has working pulse input.
#  * The full binary needs libxkbfile.so.1, absent in this container; we symlink the host's
#    copy (/run/host/usr/lib64) into .cache/android/hostlibs. (Alternative: pacman -S libxkbfile.)
#  * HOME/XDG_* are redirected into the repo so Qt settings, fontconfig cache and the modem
#    nv-ram file do not land in ~ (home-dir rule).
#
# Usage: e2e/start_emulator.sh            -> prints the adb serial on stdout (last line)
# Env:   VPM_TONE_HZ (default 440), VPM_BOOT_TIMEOUT (default 300 s)
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.config/.android/avd}"
AVD=vpm_e2e
SINK=vpm_e2e_src
TONE_HZ="${VPM_TONE_HZ:-440}"
BOOT_TIMEOUT="${VPM_BOOT_TIMEOUT:-300}"
STATE="$REPO/.cache/e2e"
EHOME="$REPO/.cache/android/home"
HOSTLIBS="$REPO/.cache/android/hostlibs"
ADB="$SDK/platform-tools/adb"
mkdir -p "$STATE" "$EHOME" "$HOSTLIBS" "$REPO/.cache/android" "$REPO/e2e/out"

log() { echo "[start_emulator] $*" >&2; }

# --- 1. tone sink -----------------------------------------------------------
if ! pactl list short sinks | awk '{print $2}' | grep -qx "$SINK"; then
  mod=$(pactl load-module module-null-sink sink_name="$SINK" sink_properties=device.description=vpm_e2e_src)
  echo "$mod" > "$STATE/sink_module"
  log "loaded null sink $SINK (module $mod)"
fi
if ! pgrep -f "^ffmpeg .*-device $SINK" >/dev/null; then
  setsid -f ffmpeg -hide_banner -loglevel error -nostdin -re -f lavfi \
    -i "sine=frequency=${TONE_HZ}:sample_rate=48000" -f pulse -device "$SINK" vpm_tone \
    >"$STATE/ffmpeg.log" 2>&1 </dev/null
  log "started ${TONE_HZ} Hz tone into $SINK"
fi

# --- 2. emulator ------------------------------------------------------------
if [ ! -e "$HOSTLIBS/libxkbfile.so.1" ]; then
  if ! ldconfig -p 2>/dev/null | grep -q 'libxkbfile.so.1 '; then
    if [ -e /run/host/usr/lib64/libxkbfile.so.1 ]; then
      ln -sf /run/host/usr/lib64/libxkbfile.so.1 "$HOSTLIBS/libxkbfile.so.1"
    else
      log "ERROR: libxkbfile.so.1 missing (install libxkbfile)"; exit 1
    fi
  fi
fi

"$ADB" start-server >/dev/null 2>&1 || true
if pgrep -f "^[^ ]*qemu-system-x86_64 .*-avd $AVD" >/dev/null; then
  log "emulator already running"
else
  EMU_LOG="$REPO/e2e/out/emulator.log"
  log "launching emulator (log: $EMU_LOG)"
  setsid -f env -u DISPLAY -u WAYLAND_DISPLAY \
    HOME="$EHOME" XDG_CONFIG_HOME="$EHOME/.config" XDG_CACHE_HOME="$EHOME/.cache" \
    XDG_DATA_HOME="$EHOME/.local/share" XDG_STATE_HOME="$EHOME/.local/state" \
    ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK" ANDROID_AVD_HOME="$AVD_HOME" \
    ANDROID_USER_HOME="$REPO/.cache/android" \
    LD_LIBRARY_PATH="$HOSTLIBS" QT_QPA_PLATFORM=offscreen TCMALLOC_SKIP_SBRK=true \
    PULSE_SOURCE="$SINK.monitor" \
    "$SDK/emulator/emulator" -avd "$AVD" -qt-hide-window -no-snapshot -no-boot-anim \
      -no-metrics -gpu swiftshader_indirect -allow-host-audio \
    >"$EMU_LOG" 2>&1 </dev/null
fi

# --- 3. wait for boot ---------------------------------------------------------
SERIAL=""
deadline=$((SECONDS + BOOT_TIMEOUT))
while [ $SECONDS -lt $deadline ]; do
  SERIAL=$("$ADB" devices | awk '/^emulator-[0-9]+\tdevice/{print $1; exit}')
  if [ -n "$SERIAL" ] && [ "$("$ADB" -s "$SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; then
    break
  fi
  # The `emulator` wrapper execs qemu after a moment; only declare death after a grace period.
  if [ $SECONDS -gt $((deadline - BOOT_TIMEOUT + 30)) ] \
     && ! pgrep -f "^[^ ]*(qemu-system-x86_64|emulator/emulator) .*-avd $AVD" >/dev/null; then
    log "ERROR: emulator died; see e2e/out/emulator.log"; exit 1
  fi
  SERIAL=""; sleep 3
done
[ -n "$SERIAL" ] || { log "ERROR: boot timeout"; exit 1; }
echo "$SERIAL" > "$STATE/serial"

# --- 4. host mic on ---------------------------------------------------------
"$ADB" -s "$SERIAL" emu avd hostmicon >/dev/null
log "booted, host mic enabled"
echo "$SERIAL"
