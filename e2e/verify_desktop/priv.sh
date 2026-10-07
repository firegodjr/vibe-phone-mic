#!/bin/bash
# Verifier wrapper around desktop/tools/pw-private.sh.
#   priv.sh start NAME   -> starts instance in $XDG_RUNTIME_DIR/NAME, writes NAME.env next to it
#   priv.sh stop NAME
#   priv.sh check NAME   -> asserts the env really points at the private daemon (exit 1 otherwise)
# Usage in scripts:  source <(priv.sh env NAME); priv.sh check NAME || exit 1
set -u
R=/home/firegodjr/repos/vibe-phone-mic; PW=$R/desktop/tools/pw-private.sh
BASE=${VPMV_BASE:-/run/user/$(id -u)}
D=$BASE/$2
case $1 in
start) rm -rf "$D"; $PW start "$D" > "$D.env" 2>"$D.err" || { cat "$D.err"; exit 1; }
       echo 'export PIPEWIRE_REMOTE=pipewire-0' >> "$D.env"; cat "$D.env" ;;
env) cat "$D.env" ;;
stop) $PW stop "$D"; rm -rf "$D" "$D.env" "$D.err" ;;
check) ( source "$D.env"
         s=$(pactl info 2>/dev/null | awk -F': ' '/Server String/{print $2}')
         [ "$s" = "unix:$D/run/pulse/native" ] || { echo "NOT PRIVATE: pulse server=$s" >&2; exit 1; }
         [ -S "$D/run/pipewire-0" ] || { echo "no private pipewire socket" >&2; exit 1; }
         [ "$(pactl get-default-sink)" = private_out ] || { echo "private default sink missing" >&2; exit 1; } ) ;;
esac
