#!/usr/bin/env bash
# Private, isolated PipeWire + pipewire-pulse + WirePlumber instance for tests.
# It never touches the user's audio daemon: own runtime dir/sockets, no hardware
# monitors (ALSA/Bluetooth/V4L2 disabled), no session D-Bus.
#
#   pw-private.sh start DIR   # start (DIR is created); prints env lines to eval
#   pw-private.sh env DIR     # print the env for clients of that instance
#   pw-private.sh stop DIR    # stop it
#
# Plugins: uses the system PipeWire if it has audio support (audioconvert,
# protocol-pulse, wireplumber); otherwise extra files from VPM_PW_ROOT
# (default: <repo>/.cache/pw-private/root, see tools/fetch-pw-test-deps.sh).
set -euo pipefail

cmd="${1:-}"; D="${2:-}"
[ -n "$cmd" ] && [ -n "$D" ] || { echo "usage: $0 start|stop|env DIR" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${VPM_PW_ROOT:-$HERE/../../.cache/pw-private/root}"

find_dir() { for d in "$@"; do [ -d "$d" ] && { echo "$d"; return; }; done; }
SYS_SPA="$(find_dir /usr/lib64/spa-0.2 /usr/lib/spa-0.2 /usr/lib/x86_64-linux-gnu/spa-0.2)"
SYS_MOD="$(find_dir /usr/lib64/pipewire-0.3 /usr/lib/pipewire-0.3 /usr/lib/x86_64-linux-gnu/pipewire-0.3)"
SYS_SHARE=/usr/share/pipewire

print_env() {
    cat <<EOF
export XDG_RUNTIME_DIR="$D/run"
export PIPEWIRE_RUNTIME_DIR="$D/run"
export PULSE_RUNTIME_PATH="$D/run/pulse"
export PULSE_SERVER="unix:$D/run/pulse/native"
export SPA_PLUGIN_DIR="$D/spa"
export PIPEWIRE_MODULE_DIR="$D/modules"
export PIPEWIRE_CONFIG_DIR="$D/pwconf"
EOF
}

case "$cmd" in
env) print_env ;;
stop)
    for f in "$D"/pids/*; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true; done
    sleep 0.3
    for f in "$D"/pids/*; do [ -f "$f" ] && kill -9 "$(cat "$f")" 2>/dev/null || true; rm -f "$f"; done
    ;;
start)
    mkdir -p "$D"/{run,spa,modules,pwconf,pids,logs,config/wireplumber/wireplumber.conf.d,state,data}
    chmod 700 "$D/run"
    # Merge plugin/module dirs (system first, extras fill the gaps).
    for src in "$SYS_SPA" "$ROOT/usr/lib/spa-0.2"; do
        [ -d "$src" ] && for p in "$src"/*; do [ -e "$D/spa/$(basename "$p")" ] || ln -s "$p" "$D/spa/"; done
    done
    for src in "$SYS_MOD" "$ROOT/usr/lib/pipewire-0.3"; do
        [ -d "$src" ] && for p in "$src"/*; do [ -e "$D/modules/$(basename "$p")" ] || ln -s "$p" "$D/modules/"; done
    done
    for src in "$SYS_SHARE" "$ROOT/usr/share/pipewire"; do
        [ -d "$src" ] && for p in "$src"/*; do [ -e "$D/pwconf/$(basename "$p")" ] || cp -r "$p" "$D/pwconf/"; done
    done
    [ -d "$D/spa/audioconvert" ] || { echo "no audioconvert SPA plugin (run tools/fetch-pw-test-deps.sh)" >&2; exit 3; }
    [ -e "$D/modules/libpipewire-module-protocol-pulse.so" ] || { echo "no protocol-pulse module" >&2; exit 3; }
    WP="$(command -v wireplumber || true)"; WPLIB=""; WPDATA=""; WPMOD=""
    if [ -z "$WP" ] && [ -x "$ROOT/usr/bin/wireplumber" ]; then
        WP="$ROOT/usr/bin/wireplumber"; WPLIB="$ROOT/usr/lib"
        WPDATA="$ROOT/usr/share/wireplumber"; WPMOD="$ROOT/usr/lib/wireplumber-0.5"
    fi
    [ -n "$WP" ] || { echo "no wireplumber" >&2; exit 3; }
    cat > "$D/config/wireplumber/wireplumber.conf.d/90-private.conf" <<'EOF'
wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez.midi = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
    support.reserve-device = disabled
    support.logind = disabled
    support.portal-permissionstore = disabled
  }
}
EOF
    eval "$(print_env)"
    export XDG_CONFIG_HOME="$D/config" XDG_STATE_HOME="$D/state" XDG_DATA_HOME="$D/data"
    unset DBUS_SESSION_BUS_ADDRESS PIPEWIRE_REMOTE
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$D/run/no-dbus"
    pipewire > "$D/logs/pipewire.log" 2>&1 & echo $! > "$D/pids/pipewire"
    for _ in $(seq 100); do [ -S "$D/run/pipewire-0" ] && break; sleep 0.05; done
    [ -S "$D/run/pipewire-0" ] || { echo "pipewire did not start" >&2; cat "$D/logs/pipewire.log" >&2; exit 4; }
    pipewire -c pipewire-pulse.conf > "$D/logs/pipewire-pulse.log" 2>&1 & echo $! > "$D/pids/pipewire-pulse"
    (
        [ -n "$WPLIB" ] && export LD_LIBRARY_PATH="$WPLIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        [ -n "$WPDATA" ] && export WIREPLUMBER_DATA_DIR="$WPDATA"
        export WIREPLUMBER_CONFIG_DIR="$D/config/wireplumber${WPDATA:+:$WPDATA}:/usr/share/wireplumber"
        [ -n "$WPMOD" ] && export WIREPLUMBER_MODULE_DIR="$WPMOD"
        exec "$WP" > "$D/logs/wireplumber.log" 2>&1
    ) & echo $! > "$D/pids/wireplumber"
    for _ in $(seq 100); do [ -S "$D/run/pulse/native" ] && break; sleep 0.05; done
    [ -S "$D/run/pulse/native" ] || { echo "pipewire-pulse did not start" >&2; cat "$D/logs/pipewire-pulse.log" >&2; exit 4; }
    # A null sink as the private default output (drives the graph, target for distractors).
    for _ in $(seq 40); do pactl info >/dev/null 2>&1 && break; sleep 0.05; done
    pactl load-module module-null-sink sink_name=private_out >/dev/null
    pactl set-default-sink private_out
    print_env
    ;;
*) echo "unknown command $cmd" >&2; exit 2 ;;
esac
