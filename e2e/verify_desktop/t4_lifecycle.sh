#!/bin/bash
# Virtual-mic lifecycle: SIGINT / SIGTERM / SIGHUP / SIGKILL + stale cleanup, defaults untouched.
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out
mods() { pactl list short modules | grep -c 'vibe_phone_mic'; }
DS0=$(pactl get-default-source); DK0=$(pactl get-default-sink)
python3 $R/e2e/verify_desktop/phone_sim.py --port 48794 >/dev/null 2>&1 & S=$!; sleep 0.4
for sig in INT TERM HUP KILL; do
  $A --headless --connect 127.0.0.1:48794 --duration 30 >$O/l_$sig.out 2>&1 & P=$!
  sleep 2; before=$(mods)
  # AppImage runtime execs the binary: signal the real process too (pkill by name) to mirror a user Ctrl+C on the process group
  kill -$sig $P; t0=$(date +%s.%N); wait $P; rc=$?; t1=$(date +%s.%N)
  sleep 0.3
  echo "SIG$sig: modules_before=$before exit=$rc stop_time=$(echo "$t1-$t0"|bc) modules_after=$(mods) summary: $(grep SUMMARY $O/l_$sig.out | cut -c1-60)"
done
pgrep -fa 'vibe-phone-mic|VibePhoneMic' | grep -v pgrep
echo "leftover after SIGKILL: $(mods)"; pactl list short modules | grep vibe_phone_mic
$A --headless --connect 127.0.0.1:48794 --duration 2 >$O/l_rerun.out 2>&1; echo "rerun exit=$?"; cat $O/l_rerun.out | grep -v HELLO
echo "after rerun: $(mods)"
echo "default source same: $([ "$DS0" = "$(pactl get-default-source)" ] && echo yes || echo NO)  sink same: $([ "$DK0" = "$(pactl get-default-sink)" ] && echo yes || echo NO)"
kill $S
