#!/bin/bash
# Edge cases against independent phone_sim. Usage: ./t3_edges.sh
unset DISPLAY WAYLAND_DISPLAY
R=/home/firegodjr/repos/vibe-phone-mic; A=$R/dist/VibePhoneMic-x86_64.AppImage; O=$R/e2e/verify_desktop/out; mkdir -p $O
run() { # name simargs -- appargs
  local name=$1; shift; local sargs=(); while [ "$1" != "--" ]; do sargs+=("$1"); shift; done; shift
  python3 $R/e2e/verify_desktop/phone_sim.py --port 48793 "${sargs[@]}" >/dev/null 2>$O/e_$name.sim & local S=$!
  sleep 0.4
  local t0=$(date +%s.%N)
  $A --headless --connect 127.0.0.1:48793 --wav $O/e_$name.wav "$@" >$O/e_$name.out 2>$O/e_$name.err; local rc=$?
  local t1=$(date +%s.%N)
  kill $S 2>/dev/null; wait $S 2>/dev/null
  echo "=== $name exit=$rc elapsed=$(echo "$t1-$t0"|bc)s"; grep -v '^HELLO\|^CONNECTED' $O/e_$name.out; sed 's/^/  err: /' $O/e_$name.err
}
run r44100 --rate 44100 --freq 700 -- --duration 3 --no-virtual-mic
python3 $R/e2e/analyze_tone.py $O/e_r44100.wav --expect 700
run r44100mic --rate 44100 --freq 700 -- --duration 3
run stereo --channels 2 --freq 600 -- --duration 2 --no-virtual-mic
run mute --behavior mute:100 -- --duration 3 --no-virtual-mic
python3 $R/e2e/verify_desktop/gaps.py $O/e_mute.wav
run volume --behavior volume:100 -- --duration 3 --no-virtual-mic
run close --behavior close:150 -- --duration 6 --no-virtual-mic
run garbage --behavior garbage -- --duration 3 --no-virtual-mic
run nohello --behavior nohello -- --duration 3 --no-virtual-mic
run oversize --behavior oversize -- --duration 3 --no-virtual-mic
run exact1mib --behavior exact1mib -- --duration 2 --no-virtual-mic
run stall --behavior stall:100 -- --duration 12 --no-virtual-mic
echo "=== refused"; $A --headless --connect 127.0.0.1:48799 --duration 2 --no-virtual-mic; echo "exit=$?"
echo "=== unresolvable"; $A --headless --connect nonexistent.invalid --duration 2 --no-virtual-mic; echo "exit=$?"
echo "=== blackhole (10.255.255.1)"; t0=$(date +%s); $A --headless --connect 10.255.255.1:48799 --duration 2 --no-virtual-mic; echo "exit=$? took $(( $(date +%s)-t0 ))s"
for args in "--headless" "--headless --connect 300.1.1.1" "--headless --connect 1.2.3.4:99999" "--headless --connect 1.2.3.4:0" "--bogus" "--headless --connect 1.2.3.4 --duration abc" "--headless --connect"; do
  $A $args >/dev/null 2>$O/e_args.err; echo "args [$args] exit=$? : $(head -1 $O/e_args.err)"; done
