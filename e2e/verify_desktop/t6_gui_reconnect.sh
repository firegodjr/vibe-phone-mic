#!/bin/bash
# GUI reconnect policy via the offscreen --screenshot path (real VpmApp logic), against phone_sim.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out
shot() { $A --screenshot $O/$1.png --connect 127.0.0.1:$2 --no-scan --no-virtual-mic --screenshot-scale 1 --screenshot-delay-ms $3 ${4:+--theme $4} 2>/dev/null; }
streams() { grep -c 'handshake.*56504d4301000000' $1; }
echo "a) two GUIs, phone sends BYE replaced"
python3 phone_sim.py --port 48770 --name "Shared Sim" >/dev/null 2>$O/g_a.err & S=$!; sleep 0.3
shot g_a1 48770 12000 & P1=$!; sleep 1; shot g_a2 48770 10000 dark & P2=$!; wait $P1 $P2; kill $S
echo "   stream handshakes in 12 s: $(streams $O/g_a.err) (expect 2), BYE replaced sent: $(grep -c 'sent BYE replaced' $O/g_a.err)"
echo "b) two GUIs, OLD phone without BYE (quick-drop limiter)"
python3 phone_sim.py --port 48771 --name "Old Sim" --no-bye >/dev/null 2>$O/g_b.err & S=$!; sleep 0.3
shot g_b1 48771 20000 & P1=$!; sleep 1; shot g_b2 48771 18000 & P2=$!; wait $P1 $P2; kill $S
echo "   stream handshakes in 20 s: $(streams $O/g_b.err)"
echo "c) BYE stopped after 4 s of streaming -> keeps retrying"
python3 phone_sim.py --port 48772 --name "Stopping Sim" --behavior bye-stopped:400 >/dev/null 2>$O/g_c.err & S=$!; sleep 0.3
shot g_c 48772 14500; kill $S; echo "   stream handshakes in 14.5 s: $(streams $O/g_c.err) (expect ~3)"
echo "d) 3 quick drops after HELLO -> stops"
python3 phone_sim.py --port 48773 --name "Flaky Sim" --behavior close:50 >/dev/null 2>$O/g_d.err & S=$!; sleep 0.3
shot g_d 48773 14000; kill $S; echo "   stream handshakes in 14 s: $(streams $O/g_d.err) (expect 3)"
echo "e) first connect refused"; shot g_e 48779 4000
echo "f) garbage server"
python3 phone_sim.py --port 48774 --behavior garbage >/dev/null 2>$O/g_f.err & S=$!; sleep 0.3
shot g_f 48774 5000 dark; kill $S; echo "   handshakes: $(streams $O/g_f.err)"
echo "g) phone killed mid-stream and restarted 3 s later -> reconnects"
python3 phone_sim.py --port 48775 --name "Restart Sim" >/dev/null 2>/dev/null & S=$!; sleep 0.3
shot g_g 48775 10000 & P=$!; sleep 4; kill $S; wait $S 2>/dev/null; sleep 3
python3 phone_sim.py --port 48775 --name "Restart Sim" >/dev/null 2>/dev/null & S=$!; wait $P; kill $S
