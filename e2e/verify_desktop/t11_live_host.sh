#!/bin/bash
# GENTLE live-session check (run from the toolbox): AppImage on the HOST against phone_sim, record the
# source on the host, inspect how clients see it, confirm it vanishes and defaults are unchanged.
R=/home/firegodjr/repos/vibe-phone-mic; V=$R/e2e/verify_desktop; O=$V/out; A=$R/dist/VibePhoneMic-x86_64.AppImage
H="flatpak-spawn --host"
DS0=$($H pactl get-default-source); DK0=$($H pactl get-default-sink)
CD0=$(coredumpctl list 2>/dev/null | grep -c pipewire)
python3 $V/phone_sim.py --port 48762 --freq 523 >/dev/null 2>&1 & S=$!; sleep 0.3
$H env -u DISPLAY -u WAYLAND_DISPLAY $A --headless --connect 127.0.0.1:48762 --duration 9 > $O/live.out 2>&1 &
P=$!
for t in $(seq 50); do $H pactl list short sources | grep -q 'vibe_phone_mic' && break; sleep 0.1; done
sleep 1
$H python3 $V/leakcheck.py rec vibe_phone_mic 4 $O/live_rec.raw --tone 523
python3 $V/clicks.py $O/live_rec.raw 523
echo "--- pactl list sources entry"
$H pactl list sources | awk '/^Source #/{blk=$0; show=0} /Name: vibe_phone_mic$/{show=1; print blk} show' | grep -E "Source #|Name:|Description|Driver|Sample Spec|Channel Map|Monitor of|Flags|State|device\.|media\.class|node\.(name|virtual|nick)|application.name|Active Port" 
echo "--- pw-dump node props"
$H pw-dump | python3 -c "
import json,sys
for o in json.load(sys.stdin):
    p=o.get('info',{}).get('props',{})
    if p.get('node.name')=='vibe_phone_mic': print({k:p[k] for k in p if k.split('.')[0] in ('media','node','device','priority','application','client')})"
wait $P; echo "app rc=$?"; grep -E "VIRTUAL_MIC|SUMMARY|warning|legacy" $O/live.out
kill $S
sleep 0.5
echo "source after exit: $($H pactl list short sources | grep -c vibe_phone_mic); nodes: $($H pw-dump | grep -c '"node.name": "vibe_phone_mic"')"
echo "defaults unchanged: $([ "$DS0" = "$($H pactl get-default-source)" ] && [ "$DK0" = "$($H pactl get-default-sink)" ] && echo yes || echo NO)"
echo "pipewire coredumps before=$CD0 after=$(coredumpctl list 2>/dev/null | grep -c pipewire)"
