#!/bin/bash
# Discovery: UDP broadcast, /24 sweep, dedupe, timing. LAN IP auto-detected.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out
LAN=$(ip -4 -o addr show scope global | awk '$4 ~ /\/24$/ {split($4,a,"/"); print a[1]; exit}')
echo "LAN=$LAN"
scan() { local t0=$(date +%s.%N); local j=$($A --scan "$@" 2>/dev/null); echo "  rc=$? took=$(echo "$(date +%s.%N)-$t0"|bc)s $j"; }
echo "1) nothing of ours running (127.0.0.1:48777 = adb forward to emulator)"; scan
echo "2) UDP-only sim (udp 0.0.0.0:48777, advertises tcp port 48790 on LAN ip)"
python3 phone_sim.py --host $LAN --port 48790 --udp-port 48777 --name "UDP Sim" >/dev/null 2>$O/d2.err & S=$!; sleep 0.4
scan; scan --timeout-ms 500; kill $S; wait $S 2>/dev/null; grep -c "udp from" $O/d2.err
echo "3) TCP-only sim on $LAN:48777 (no UDP) -> sweep"
python3 phone_sim.py --host $LAN --port 48777 --name "Sweep Sim" >/dev/null 2>$O/d3.err & S=$!; sleep 0.4
scan; scan --timeout-ms 800; kill $S; wait $S 2>/dev/null; echo "  probes seen: $(grep -c handshake $O/d3.err), stream handshakes: $(grep -c '0100 *$\|01000000$' $O/d3.err)"
echo "4) UDP + TCP on $LAN:48777 -> dedupe"
python3 phone_sim.py --host $LAN --port 48777 --udp-port 48777 --name "Both Sim" >/dev/null 2>$O/d4.err & S=$!; sleep 0.4
scan; kill $S; wait $S 2>/dev/null
echo "5) UDP reply with garbage / wrong service ignored"
python3 - <<'PY' & S=$!
import socket
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); u.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); u.bind(("0.0.0.0",48777)); u.settimeout(4)
try:
  while True:
    d,src=u.recvfrom(100); u.sendto(b"\xff\xfegarbage",src); u.sendto(b'{"service":"other","port":1}',src); u.sendto(b'{"service":"vibe-phone-mic","port":70000}',src)
except Exception: pass
PY
sleep 0.3; scan; wait $S
