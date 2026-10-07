#!/bin/bash
# System-audio leak check: quiet 1 kHz distractor on the default sink while recording vibe_phone_mic
# in every app lifecycle state. Never touches defaults; streams only to 4877x ports.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; V=$R/e2e/verify_desktop; O=$V/out
DS0=$(pactl get-default-source); DK0=$(pactl get-default-sink)
LC="python3 $V/leakcheck.py"
summ() { python3 -c "
import json,sys
d=json.loads(sys.stdin.read() or '{}')
if 'error' in d: print('   ERROR', d['error']); sys.exit()
t=[k for k in d if k.startswith('amp_') and k!='amp_1000']
print('   %-34s secs=%.1f nonzero=%d peak=%.4f amp1000=%.2e %s feeders=%s inputs=%s same_src=%s'%('$1',d['secs'],d['nonzero_samples'],d['peak'],d['amp_1000'],' '.join('%s=%.3f'%(k,d[k]) for k in t),d['feeding_links'],d['mic_input_ports'],d['source_still_same']))"; }
mon() { $LC rec @DEFAULT_MONITOR@ 1.2 $O/k_mon.raw | python3 -c "import json,sys; d=json.load(sys.stdin); print('   [distractor on speakers: monitor amp1000=%.3f]'%d['amp_1000'])"; }
$LC distract 170 & D=$!
for i in $(seq 40); do a=$($LC rec @DEFAULT_MONITOR@ 0.8 $O/k_mon.raw | python3 -c "import json,sys; print(json.load(sys.stdin)['amp_1000'])"); python3 -c "import sys; sys.exit(0 if $a>0.01 else 1)" && break; done
mon
echo "A) GUI lifetime, mic up, no phone"
$A --screenshot $O/k_a.png --no-scan --screenshot-delay-ms 9000 2>/dev/null & G=$!
sleep 2.5; $LC rec vibe_phone_mic 4 $O/k_a.raw | summ "idle, never connected"; wait $G
echo "B) streaming 523 Hz (headless)"
python3 $V/phone_sim.py --port 48776 --freq 523 >/dev/null 2>&1 & S=$!; sleep 0.3
$A --headless --connect 127.0.0.1:48776 --duration 7 >/dev/null 2>&1 & H=$!
sleep 1.5; $LC rec vibe_phone_mic 4 $O/k_b.raw --tone 523 | summ "streaming"; wait $H; kill $S
echo "C) GUI: connect -> phone drops -> reconnect (close:300 = 3 s sessions)"
python3 $V/phone_sim.py --port 48776 --freq 523 --behavior close:300 >/dev/null 2>&1 & S=$!; sleep 0.3
$A --screenshot $O/k_c.png --no-scan --connect 127.0.0.1:48776 --screenshot-delay-ms 13000 2>/dev/null & G=$!
sleep 1.5; $LC rec vibe_phone_mic 10 $O/k_c.raw --tone 523 | summ "across drop+reconnect"; wait $G; kill $S
echo "D) GUI: phone killed abruptly (SIGKILL) -> idle/retrying"
python3 $V/phone_sim.py --port 48776 --freq 523 >/dev/null 2>&1 & S=$!; sleep 0.3
$A --screenshot $O/k_d.png --no-scan --connect 127.0.0.1:48776 --screenshot-delay-ms 11000 2>/dev/null & G=$!
sleep 2; kill -9 $S; sleep 1; $LC rec vibe_phone_mic 5 $O/k_d.raw | summ "after phone SIGKILL (retrying)"; wait $G
echo "E) second instance starts and exits while first (GUI, idle) runs"
$A --screenshot $O/k_e.png --no-scan --screenshot-delay-ms 12000 2>/dev/null & G=$!
sleep 2; python3 $V/phone_sim.py --port 48769 --freq 880 >/dev/null 2>&1 & S=$!; sleep 0.3
$A --headless --connect 127.0.0.1:48769 --duration 3 > $O/k_e2.out 2>&1 &
H=$!; sleep 0.5; $LC rec vibe_phone_mic 2.3 $O/k_e1.raw --tone 880 | summ "during 2nd instance"; wait $H; kill $S
echo "   2nd: $(head -1 $O/k_e2.out | cut -c1-80)"
$LC rec vibe_phone_mic 3 $O/k_e3.raw --tone 880 | summ "after 2nd instance exit"; wait $G
echo "F) AppImage SIGKILL while streaming -> orphan module; then restart"
python3 $V/phone_sim.py --port 48776 --freq 523 >/dev/null 2>&1 & S=$!; sleep 0.3
$A --headless --connect 127.0.0.1:48776 --duration 60 >/dev/null 2>&1 & H=$!
sleep 2; kill -9 $H; wait $H 2>/dev/null; sleep 0.5
echo "   orphans: $(pactl list short modules | grep -c 'source_name=vibe_phone_mic') module(s), fifos: $(ls $XDG_RUNTIME_DIR | grep -c 'vibe-phone-mic-.*fifo')"
$LC rec vibe_phone_mic 3 $O/k_f1.raw --tone 523 | summ "orphan module (no writer)"
$A --headless --connect 127.0.0.1:48776 --duration 6 > $O/k_f2.out 2>&1 & H=$!
sleep 1.5; $LC rec vibe_phone_mic 3.5 $O/k_f2.raw --tone 523 | summ "after restart (streaming)"; wait $H; kill $S
grep -m1 removed $O/k_f2.out | sed 's/^/   /'
echo "   after exit: modules=$(pactl list short modules | grep -c vibe_phone_mic) fifos=$(ls $XDG_RUNTIME_DIR | grep -c 'vibe-phone-mic-.*fifo')"
mon
kill $D 2>/dev/null; pkill -P $D 2>/dev/null; wait $D 2>/dev/null
echo "defaults unchanged: $([ "$DS0" = "$(pactl get-default-source)" ] && [ "$DK0" = "$(pactl get-default-sink)" ] && echo yes || echo NO)"
