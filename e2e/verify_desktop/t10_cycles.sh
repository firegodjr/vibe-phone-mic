#!/bin/bash
# Client-side stress on a PRIVATE PipeWire.
#   t10_cycles.sh startstop NAME  : 100 app start/stop cycles (mixed: normal exit, SIGTERM, SIGINT, SIGKILL at random times)
#   t10_cycles.sh connects  NAME  : one long-lived GUI; phone BYE-stops every 3.2 s -> 100 connect/disconnect cycles
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; V=$R/e2e/verify_desktop; O=$V/out; A=$R/dist/VibePhoneMic-x86_64.AppImage
MODE=$1; NAME=$2; source <($V/priv.sh env $NAME); $V/priv.sh check $NAME || exit 1
D=/run/user/$(id -u)/$NAME
PWPID=$(cat $D/pids/pipewire); PPPID=$(cat $D/pids/pipewire-pulse)
nodes() { pw-dump 2>/dev/null | grep -c '"node.name": "vibe_phone_mic"'; }
clients() { pw-dump 2>/dev/null | grep -c '"application.name": "Vibe Phone Mic"'; }
echo "[$MODE] private pipewire pid=$PWPID pulse=$PPPID; baseline objects=$(pw-dump | grep -c '"id":')"
if [ "$MODE" = startstop ]; then
  python3 $V/phone_sim.py --port 48764 --freq 523 >/dev/null 2>&1 & S=$!; sleep 0.3
  fails=0; t0=$(date +%s)
  for i in $(seq 100); do
    r=$((RANDOM % 4)); dly=$(python3 -c "import random; print(round(random.uniform(0.02,0.6),3))")
    case $r in
      0) timeout 10 $A --headless --connect 127.0.0.1:48764 --duration $dly >/dev/null 2>&1; rc=$? ;;
      *) $A --headless --connect 127.0.0.1:48764 --duration 30 >/dev/null 2>&1 & P=$!; sleep $dly
         sig=$(case $r in 1) echo TERM;; 2) echo INT;; 3) echo KILL;; esac); kill -$sig $P; 
         for t in $(seq 100); do kill -0 $P 2>/dev/null || break; sleep 0.05; done
         kill -0 $P 2>/dev/null && { echo "   cycle $i: app HUNG after SIG$sig"; kill -9 $P; fails=$((fails+1)); }
         wait $P 2>/dev/null; rc=$? ;;
    esac
    n=0; for t in $(seq 40); do n=$(nodes); [ "$n" = 0 ] && break; sleep 0.05; done
    [ "$n" != 0 ] && { echo "   cycle $i: leaked node(s): $n"; fails=$((fails+1)); }
    kill -0 $PWPID 2>/dev/null || { echo "   cycle $i: PRIVATE PIPEWIRE DIED"; fails=$((fails+1)); break; }
  done
  kill $S
  echo "   100 cycles in $(( $(date +%s)-t0 ))s, failures=$fails"
else
  python3 $V/phone_sim.py --port 48763 --freq 523 --behavior bye-stopped:320 >$O/c_sim.out 2>$O/c_sim.err & S=$!; sleep 0.3
  $A --screenshot $O/c_end.png --no-scan --connect 127.0.0.1:48763 --screenshot-scale 1 --screenshot-delay-ms ${SECS:-560}000 > $O/c_gui.log 2>&1 & G=$!
  for k in $(seq 1 ${SAMPLES:-6}); do sleep $(( ${SECS:-560} / ${SAMPLES:-6} - 5 ));
    python3 $V/leakcheck.py rec vibe_phone_mic 4 $O/c_rec$k.raw --tone 523 | python3 -c "import json,sys; d=json.load(sys.stdin); print('   sample $k: streams so far=%s nodes=%s amp523=%.3f nonzero=%d feeders=%s'%('$(grep -c 'handshake.*56504d4301000000' $O/c_sim.err)','$(nodes)',d.get('amp_523',-1),d.get('nonzero_samples',-1),d.get('feeding_links')))"
  done
  wait $G; echo "   gui rc=$? ; total stream connections=$(grep -c 'handshake.*56504d4301000000' $O/c_sim.err)"; kill $S
fi
echo "   pipewire alive: $(kill -0 $PWPID 2>/dev/null && echo yes || echo NO) pulse alive: $(kill -0 $PPPID 2>/dev/null && echo yes || echo NO); nodes=$(nodes) app-clients=$(clients) objects=$(pw-dump | grep -c '"id":')"
