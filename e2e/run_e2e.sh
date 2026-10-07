#!/usr/bin/env bash
# End-to-end test: real APK in the headless emulator  ->  real AppImage on this host.
#
# The emulator's microphone is fed a 440 Hz tone from an isolated PulseAudio null sink
# (see start_emulator.sh), so every check below exercises the REAL capture path
# (AudioRecord -> phone server -> TCP -> AppImage -> WAV and the "Vibe Phone Mic" source).
#
# Usage: e2e/run_e2e.sh [--no-build] [--keep-emulator]
# Output: e2e/out/e2e-*.{wav,log,png} and a PASS/FAIL table; exit 0 only if all checks pass.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$REPO/e2e/out"; mkdir -p "$OUT"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
ADB="$SDK/platform-tools/adb"
APPIMAGE="$REPO/dist/VibePhoneMic-x86_64.AppImage"
APK="$REPO/android/app/build/outputs/apk/debug/app-debug.apk"
PKG=com.vibephonemic
PORT=48777
TONE=440
BUILD=1; KEEP=0
for a in "$@"; do case "$a" in --no-build) BUILD=0;; --keep-emulator) KEEP=1;; *) echo "unknown arg $a"; exit 2;; esac; done

# Home-dir rule: keep every cache inside the repo.
export CARGO_HOME="$REPO/.cache/cargo"
export GRADLE_USER_HOME="$REPO/.cache/gradle"
export ANDROID_PREFS_ROOT="$REPO/.cache/android-prefs"
export ANDROID_USER_HOME="$REPO/.cache/android-prefs/.android"
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}"
export ANDROID_HOME="$SDK"
unset DISPLAY WAYLAND_DISPLAY   # everything headless; the screen may be off

RESULTS=()
pass() { RESULTS+=("PASS  $1"); echo "[e2e] PASS  $1"; }
fail() { RESULTS+=("FAIL  $1"); echo "[e2e] FAIL  $1"; }
check() { local name="$1"; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }
log() { echo "[e2e] $*"; }

# `adb emu` needs the console auth token the emulator wrote under its redirected HOME.
emu() { HOME="$REPO/.cache/android/home" ANDROID_USER_HOME="$REPO/.cache/android" "$ADB" -s "$SERIAL" emu "$@"; }
A() { "$ADB" -s "$SERIAL" "$@"; }
ui() { ANDROID_SERIAL="$SERIAL" "$REPO/e2e/ui.sh" "$@"; }
analyze() { python3 "$REPO/e2e/analyze_tone.py" "$@"; }

# Fraction of 10 ms blocks that are exactly zero (detects "frames with no content").
zero_blocks() {
  python3 - "$1" <<'EOF'
import sys, wave, array
w = wave.open(sys.argv[1]); n = w.getnframes(); ch = w.getnchannels(); rate = w.getframerate()
a = array.array('h', w.readframes(n)); blk = rate // 100 * ch
blocks = [a[i:i+blk] for i in range(0, len(a) - blk + 1, blk)]
z = sum(1 for b in blocks if not any(b))
print(f"{z} {len(blocks)}")
EOF
}
rms_of() { python3 -c '
import sys,wave,array,math
w=wave.open(sys.argv[1]);a=array.array("h",w.readframes(w.getnframes()))
a=a[len(a)//5:]  # skip connection warm-up
print(math.sqrt(sum(x*x for x in a)/max(1,len(a)))/32768)' "$1"; }

# Bring the app to front, run a UI action, then go HOME so the mic HAL isn't starved
# (the emulator's mic wedges under foreground UI load - see PLAN.md emulator notes).
with_app() {
  A shell am start -n "$PKG/.MainActivity" >/dev/null; sleep 2
  "$@"; sleep 1
  A shell input keyevent KEYCODE_HOME; sleep 1
}

# headless_run NAME SECS [extra args] -> runs the AppImage, leaves $OUT/e2e-NAME.{wav,log}
headless_run() {
  local name="$1" secs="$2"; shift 2
  "$APPIMAGE" --headless --connect "127.0.0.1:$PORT" --wav "$OUT/e2e-$name.wav" --duration "$secs" "$@" \
    >"$OUT/e2e-$name.log" 2>&1
  echo $? >"$OUT/e2e-$name.exit"
}

# Stream once and retry if the emulator mic HAL wedged during the window.
real_tone_run() {
  local name="$1" secs="$2" try
  for try in 1 2 3; do
    A logcat -c
    headless_run "$name" "$secs"
    local wedged; wedged=$(A logcat -d | grep -c "pcm_readi failed")
    if [ "$wedged" = 0 ]; then return 0; fi
    log "emulator mic HAL wedged during '$name' (attempt $try), restarting service and retrying"
    restart_service
  done
  return 1
}

restart_service() {
  A shell am force-stop "$PKG"
  A shell am start-foreground-service -n "$PKG/.MicService" >/dev/null
  for _ in $(seq 30); do
    python3 "$REPO/e2e/verify_android_client.py" probe "127.0.0.1:$PORT" >/dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

# ---------------------------------------------------------------- 1. build + unit suites
if [ "$BUILD" = 1 ]; then
  log "desktop: cargo test + AppImage"
  (cd "$REPO/desktop" && cargo test --quiet >"$OUT/e2e-cargo-test.log" 2>&1)
  check "desktop cargo test" [ $? = 0 ]
  (cd "$REPO/desktop" && ./build-appimage.sh >"$OUT/e2e-appimage-build.log" 2>&1)
  check "AppImage build" [ -x "$APPIMAGE" ]
  log "android: unit tests + APKs"
  (cd "$REPO/android" && ./gradlew --quiet testDebugUnitTest assembleDebug assembleRelease >"$OUT/e2e-gradle.log" 2>&1)
  check "android unit tests + build" [ $? = 0 ]
  cp "$REPO/android/app/build/outputs/apk/release/app-release.apk" "$REPO/dist/VibePhoneMic.apk" 2>/dev/null
fi

# ---------------------------------------------------------------- 2. emulator + app
SERIAL=$("$REPO/e2e/start_emulator.sh" | tail -1)
check "emulator booted ($SERIAL)" [ -n "$SERIAL" ]
[ -n "$SERIAL" ] || { printf '%s\n' "${RESULTS[@]}"; exit 1; }
A root >/dev/null 2>&1; sleep 2
emu avd hostmicon >/dev/null 2>&1
A shell device_config put privacy camera_mic_icons_enabled false
for s in window_animation_scale transition_animation_scale animator_duration_scale; do A shell settings put global $s 0; done
A shell cmd uimode night no >/dev/null

A install -r -g "$APK" >"$OUT/e2e-install.log" 2>&1
check "APK installed" grep -q Success "$OUT/e2e-install.log"
A shell pm grant "$PKG" android.permission.RECORD_AUDIO
A shell pm grant "$PKG" android.permission.POST_NOTIFICATIONS 2>/dev/null
A shell am force-stop "$PKG"
# Launch the UI the way a user would: it auto-starts the server.
A shell am start -n "$PKG/.MainActivity" >/dev/null; sleep 4
A forward "tcp:$PORT" "tcp:$PORT" >/dev/null
check "phone server auto-started on app launch" \
  python3 "$REPO/e2e/verify_android_client.py" probe "127.0.0.1:$PORT"
ui text ip_text >"$OUT/e2e-ip.txt" 2>/dev/null; ui text port_text >>"$OUT/e2e-ip.txt" 2>/dev/null
check "phone UI shows IP and port" grep -q "$PORT" "$OUT/e2e-ip.txt"
A shell input keyevent KEYCODE_HOME; sleep 1
restart_service

# ---------------------------------------------------------------- 3. discovery
"$APPIMAGE" --scan --timeout-ms 2500 >"$OUT/e2e-scan.json" 2>"$OUT/e2e-scan.log"
check "AppImage --scan finds the phone (USB/localhost)" \
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if any(x["host"]=="127.0.0.1" and x["port"]==48777 for x in d) else 1)' "$OUT/e2e-scan.json"
# UDP responder in the guest, reached through an emulator UDP redirect.
emu redir add "udp:$PORT:$PORT" >/dev/null 2>&1
python3 "$REPO/e2e/verify_android_client.py" discover --target 127.0.0.1 --port "$PORT" >"$OUT/e2e-udp.log" 2>&1
check "phone answers UDP discovery" grep -q '"vibe-phone-mic"' "$OUT/e2e-udp.log"
emu redir del "udp:$PORT" >/dev/null 2>&1

# ---------------------------------------------------------------- 4. real audio, end to end
log "streaming real mic audio (6 s) while recording the Vibe Phone Mic source"
( sleep 2.5; timeout 3 parecord -d vibe_phone_mic --raw --format=s16le --rate=48000 --channels=1 "$OUT/e2e-virtualmic.raw" ) &
PAREC=$!
real_tone_run real 6
wait $PAREC
check "headless stream exit 0" [ "$(cat "$OUT/e2e-real.exit")" = 0 ]
check "HELLO received" grep -q '^HELLO {' "$OUT/e2e-real.log"
check "STATUS received (unmuted, vol 100)" grep -qE '^STATUS .*"muted": ?false.*' "$OUT/e2e-real.log"
check "WAV carries the ${TONE} Hz tone" analyze "$OUT/e2e-real.wav" --expect $TONE --tol 5 --min-dbfs -40
read -r Z N < <(zero_blocks "$OUT/e2e-real.wav")
log "real stream: $Z of $N 10ms blocks are all-zero"
check "no empty (all-zero) frames once streaming (${Z}/${N})" [ "$Z" -le 10 -a "$N" -ge 500 ]
check "Vibe Phone Mic pulse source carries the tone" analyze "$OUT/e2e-virtualmic.raw" --expect $TONE --tol 5 --min-dbfs -40
check "virtual mic removed after exit" bash -c '! pactl list short modules | grep -q vibe_phone_mic'
BASE_RMS=$(rms_of "$OUT/e2e-real.wav")

# ---------------------------------------------------------------- 5. mute via phone UI
with_app ui tap mute_button
real_tone_run muted 4
check "STATUS reports muted" grep -qE '^STATUS .*"muted": ?true' "$OUT/e2e-muted.log"
read -r Z N < <(zero_blocks "$OUT/e2e-muted.wav")
check "muted: frames keep flowing but are silent (${Z}/${N} zero)" [ "$N" -ge 300 -a "$Z" -ge $((N - 5)) ]
with_app ui tap mute_button
real_tone_run unmuted 4
check "unmute restores the tone" analyze "$OUT/e2e-unmuted.wav" --expect $TONE --tol 5 --min-dbfs -40

# ---------------------------------------------------------------- 6. volume via phone UI
with_app bash -c 'for i in 1 2 3 4 5; do "$0" tap volume_down; sleep 0.3; done' "$REPO/e2e/ui.sh"
real_tone_run vol50 4
HALF_RMS=$(rms_of "$OUT/e2e-vol50.wav")
check "STATUS reports volume 50" grep -qE '^STATUS .*"volume": ?50' "$OUT/e2e-vol50.log"
check "volume 50% halves the level (base=$BASE_RMS half=$HALF_RMS)" \
  python3 -c "import sys; r=$HALF_RMS/$BASE_RMS; print('ratio',r); sys.exit(0 if 0.4<r<0.6 else 1)"
with_app bash -c 'for i in 1 2 3 4 5; do "$0" tap volume_up; sleep 0.3; done' "$REPO/e2e/ui.sh"

# ---------------------------------------------------------------- 7. client replacement (BYE)
"$APPIMAGE" --headless --connect "127.0.0.1:$PORT" --duration 12 --no-virtual-mic >"$OUT/e2e-replaced-a.log" 2>&1 &
APID=$!; sleep 3
headless_run replaced-b 3 --no-virtual-mic
T0=$(date +%s); wait $APID; T1=$(date +%s)
check "replaced client is told BYE replaced and stops" grep -qi 'DISCONNECTED.*replaced' "$OUT/e2e-replaced-a.log"
check "replacement client streams" analyze "$OUT/e2e-replaced-b.wav" --expect $TONE --tol 5 --min-dbfs -40

# ---------------------------------------------------------------- 8. settings: input selection
A shell am start -n "$PKG/.MainActivity" >/dev/null; sleep 2
ui tap settings_button; sleep 1
ui dump >"$OUT/e2e-settings.txt"
OPT=$(grep -o 'input_option_[0-9]\+' "$OUT/e2e-settings.txt" | head -1)
if [ -n "$OPT" ]; then
  ui tap "$OPT"; sleep 1
  A exec-out screencap -p >"$OUT/e2e-android-settings.png"
  ui tap back_button; A shell input keyevent KEYCODE_HOME; sleep 1
  real_tone_run input 4
  check "selected input reported in STATUS" grep -qE '^STATUS .*"input": ?"[^"]*[Bb]uilt-in' "$OUT/e2e-input.log"
  check "audio flows from selected input" analyze "$OUT/e2e-input.wav" --expect $TONE --tol 5 --min-dbfs -40
  A shell am start -n "$PKG/.MainActivity" >/dev/null; sleep 2
  ui tap settings_button; sleep 1; ui tap input_option_default; sleep 1; ui tap back_button
else
  fail "settings lists a selectable microphone"
fi
A shell input keyevent KEYCODE_HOME; sleep 1

# ---------------------------------------------------------------- 9. screenshots of both apps, connected to each other
log "screenshots"
"$APPIMAGE" --headless --connect "127.0.0.1:$PORT" --duration 14 --no-virtual-mic >"$OUT/e2e-shot-stream.log" 2>&1 &
SPID=$!; sleep 2
# Test-tone hook keeps the phone meter moving while the activity is in front (mic HAL limitation).
A shell am start -n "$PKG/.MainActivity" --ez test_tone true --ei test_tone_hz $TONE >/dev/null; sleep 3
A exec-out screencap -p >"$OUT/e2e-android-light-connected.png"
A shell cmd uimode night yes >/dev/null; sleep 2
A exec-out screencap -p >"$OUT/e2e-android-dark-connected.png"
A shell cmd uimode night no >/dev/null
wait $SPID
for theme in light dark; do
  "$APPIMAGE" --screenshot "$OUT/e2e-desktop-$theme-connected.png" --theme $theme --connect "127.0.0.1:$PORT" --no-virtual-mic \
    >>"$OUT/e2e-shot-desktop.log" 2>&1
done
A shell am start -n "$PKG/.MainActivity" --ez test_tone false >/dev/null; sleep 1
A shell input keyevent KEYCODE_HOME
check "screenshots written" [ -s "$OUT/e2e-desktop-light-connected.png" -a -s "$OUT/e2e-android-light-connected.png" ]

# ---------------------------------------------------------------- 10. crashes
A logcat -b crash -d >"$OUT/e2e-crash.log" 2>&1
check "no app crashes in logcat" bash -c "! grep -q '$PKG' '$OUT/e2e-crash.log'"

[ "$KEEP" = 1 ] || "$REPO/e2e/stop_emulator.sh" >/dev/null 2>&1

echo; echo "================ E2E RESULTS ================"
printf '%s\n' "${RESULTS[@]}"
FAILS=$(printf '%s\n' "${RESULTS[@]}" | grep -c '^FAIL')
echo "============ $(( ${#RESULTS[@]} - FAILS )) passed, $FAILS failed ============"
[ "$FAILS" = 0 ]
