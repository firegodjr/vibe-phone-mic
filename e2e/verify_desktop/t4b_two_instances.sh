#!/bin/bash
# Two app instances at once, and stream-client replacement by the phone.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out
python3 $R/e2e/verify_desktop/phone_sim.py --port 48795 --freq 523 >/dev/null 2>&1 & S1=$!
python3 $R/e2e/verify_desktop/phone_sim.py --port 48796 --freq 880 >/dev/null 2>&1 & S2=$!
sleep 0.4
$A --headless --connect 127.0.0.1:48795 --duration 8 >$O/two_a.out 2>$O/two_a.err & PA=$!
sleep 1.5; echo "A modules:"; pactl list short modules | grep vibe_phone_mic | cut -f1,2
$A --headless --connect 127.0.0.1:48796 --duration 3 >$O/two_b.out 2>$O/two_b.err & PB=$!
sleep 1.5; echo "with B running:"; pactl list short modules | grep vibe_phone_mic | cut -f1,2
wait $PB; echo "B exit=$?; B stderr: $(cat $O/two_b.err | tr '\n' '|')"
echo "after B exit, A still running: modules:"; pactl list short modules | grep vibe_phone_mic | cut -f1,2 || echo "  (none - A's virtual mic is gone)"
timeout 2 parecord --latency-msec=20 -d vibe_phone_mic --raw --format=s16le --rate=48000 --channels=1 $O/two_rec.raw 2>&1 | head -2
wait $PA; echo "A exit=$?"; grep -v HELLO $O/two_a.out; cat $O/two_a.err
echo "--- replacement: two clients to same phone"
$A --headless --connect 127.0.0.1:48795 --duration 6 --no-virtual-mic >$O/rep_a.out 2>&1 & PA=$!
sleep 1.5
$A --headless --connect 127.0.0.1:48795 --duration 2 --no-virtual-mic >$O/rep_b.out 2>&1; echo "B exit=$?"
wait $PA; echo "A exit=$?"; grep -E "DISCONN|SUMMARY" $O/rep_a.out $O/rep_b.out
kill $S1 $S2
