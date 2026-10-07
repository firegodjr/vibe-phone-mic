#!/bin/bash
# Verify: AppImage headless receives tone from phone_sim, WAV + virtual mic carry it.
set -u
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out; mkdir -p $O
FREQ=${FREQ:-523}; PORT=${PORT:-48791}
DS0=$(pactl get-default-source); DK0=$(pactl get-default-sink)
python3 $R/e2e/verify_desktop/phone_sim.py --port $PORT --freq $FREQ > $O/t2_sim.out 2> $O/t2_sim.err & SIM=$!
sleep 0.5
$A --headless --connect 127.0.0.1:$PORT --wav $O/t2.wav --duration ${DUR:-7} > $O/t2_app.out 2> $O/t2_app.err & APP=$!
for i in $(seq 50); do grep -q VIRTUAL_MIC $O/t2_app.out 2>/dev/null && break; sleep 0.1; done
sleep 1.5
pactl list short sources | grep vibe
timeout ${REC:-4} parecord --latency-msec=20 -d vibe_phone_mic --raw --format=s16le --rate=48000 --channels=1 $O/t2_mic.raw
wait $APP; echo "app exit=$?"
kill $SIM
cat $O/t2_app.out; cat $O/t2_app.err
python3 $R/e2e/analyze_tone.py $O/t2.wav --expect $FREQ
python3 $R/e2e/analyze_tone.py $O/t2_mic.raw --rate 48000 --expect $FREQ
echo "modules left: $(pactl list short modules | grep -c vibe_phone_mic)"
echo "default source same: $([ "$DS0" = "$(pactl get-default-source)" ] && echo yes || echo NO)  sink same: $([ "$DK0" = "$(pactl get-default-sink)" ] && echo yes || echo NO)"
