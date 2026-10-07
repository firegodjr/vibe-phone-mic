#!/bin/bash
# PipeWire restart recovery (PRIVATE instance only): GUI streaming while the private daemons are
# stopped/started 10x quickly; afterwards exactly one source with audio, app alive.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; V=$R/e2e/verify_desktop; O=$V/out; A=$R/dist/VibePhoneMic-x86_64.AppImage
NAME=${PRIV:-vpmv1}; D=/run/user/$(id -u)/$NAME; PW=$R/desktop/tools/pw-private.sh
source <($V/priv.sh env $NAME); $V/priv.sh check $NAME || exit 1
LIVE_PW=$(pgrep -x pipewire | tr '\n' ' '); echo "live pipewire pids (must not be touched): $(for p in $LIVE_PW; do [ "$(cat /proc/$p/environ 2>/dev/null | tr '\0' '\n' | grep -c vpmv)" = 0 ] && echo -n "$p "; done)"
nsrc() { pactl list short sources 2>/dev/null | grep -c 'vibe_phone_mic'; }
restart() { $PW stop $D >/dev/null 2>&1; sleep ${1:-0.3}; $PW start $D >/dev/null 2>&1 || echo "   private restart failed"; }
python3 $V/phone_sim.py --port 48765 --freq 523 --name "Restart Sim" >/dev/null 2>&1 & S=$!; sleep 0.3
echo "1) screenshot while PipeWire is DOWN"
$A --screenshot $O/r_down.png --theme light --no-scan --connect 127.0.0.1:48765 --screenshot-scale 1 --screenshot-delay-ms 6000 > $O/r_down.log 2>&1 & G=$!
sleep 3; $PW stop $D >/dev/null 2>&1; wait $G; echo "   gui rc=$? log: $(tr '\n' '|' < $O/r_down.log | cut -c1-200)"; $PW start $D >/dev/null 2>&1
echo "2) 10 quick restarts while GUI streams"
$A --screenshot $O/r_after.png --theme dark --no-scan --connect 127.0.0.1:48765 --screenshot-scale 1 --screenshot-delay-ms 70000 > $O/r_loop.log 2>&1 & G=$!
for t in $(seq 50); do [ "$(nsrc)" = 1 ] && break; sleep 0.1; done
for i in $(seq 10); do
  sleep 1.5; restart 0.2; t0=$(date +%s.%N)
  for t in $(seq 100); do [ "$(nsrc)" -ge 1 ] && break; sleep 0.1; done
  echo "   restart $i: source back after $(echo "$(date +%s.%N)-$t0"|bc | cut -c1-4)s, count=$(nsrc), app alive=$(kill -0 $G 2>/dev/null && echo y || echo n)"
done
sleep 3
echo "   sources=$(nsrc) pw-nodes=$(pw-dump 2>/dev/null | grep -c '"node.name": "vibe_phone_mic"')"
python3 $V/leakcheck.py rec vibe_phone_mic 4 $O/r_rec.raw --tone 523 | python3 -c "import json,sys; d=json.load(sys.stdin); print('   after restarts: amp523=%.3f amp1000=%.1e nonzero=%d feeders=%s'%(d.get('amp_523',-1),d.get('amp_1000',-1),d.get('nonzero_samples',-1),d.get('feeding_links')) if 'error' not in d else d)"
python3 $V/clicks.py $O/r_rec.raw 523 | sed 's/^/   /'
wait $G; echo "   gui rc=$?; reconnect log lines: $(grep -c 'reconnecting' $O/r_loop.log) lost / $(grep -c 'is back' $O/r_loop.log) back"
echo "   sources after exit=$(nsrc)"
kill $S
