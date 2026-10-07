# Vibe Phone Mic — desktop (Linux)

Receives microphone audio from the Vibe Phone Mic Android app (protocol v1, see `../PLAN.md`) and exposes it as a
virtual microphone called **Vibe Phone Mic** in PulseAudio/PipeWire.

## Use

```sh
./VibePhoneMic-x86_64.AppImage            # GUI: pick a phone (or type its IP), press Connect
```

Then select **Vibe Phone Mic** as the microphone in your apps. Mute and volume are controlled on the phone; the
desktop only shows them. For USB, run `adb forward tcp:48777 tcp:48777` and connect to `127.0.0.1`.

CLI (no window or display needed):

```sh
vibe-phone-mic --scan [--timeout-ms 2000]          # JSON array of phones found
vibe-phone-mic --headless --connect HOST[:PORT] [--wav out.wav] [--duration SECS] [--no-virtual-mic]
vibe-phone-mic --screenshot out.png [--theme light|dark] [--connect HOST] [--no-scan]   # offscreen render of the GUI
```

Headless mode prints machine-readable lines on stdout:
`CONNECTED`, `HELLO {json}`, `STATUS {json}`, `VIRTUAL_MIC ready|error`, `DISCONNECTED reason` and finally
`SUMMARY frames=N samples=N rms=X dominant_hz=Y lost_frames=N muted=B`.
Exit codes: 0 = audio received, 1 = no audio (connection failed), 2 = bad arguments.

**Virtual mic (PipeWire).** "Vibe Phone Mic" is a native PipeWire stream that is itself an `Audio/Source` node
(`node.name=vibe_phone_mic`, `node.virtual=true`, `priority.session=1` so it never becomes the default input unless
you pick it), 48 kHz mono S16. No PulseAudio modules are loaded at all, so:
- nothing can fall back to the default sink's monitor (system audio), and no extra sink appears in output lists;
- nothing has to be unloaded on exit (unloading `module-pipe-source` crashes PipeWire 1.6.8);
- if the app dies (even SIGKILL), PipeWire removes the node by itself.

The app downmixes/resamples phone audio to 48 kHz mono (windowed-sinc) and the realtime process callback pulls from a
40 ms jitter buffer: silence while buffering or on underrun, oldest audio dropped on overflow (250 ms hard cap; the
queue is trimmed back if it stays above 80 ms for 2 s). Measured end-to-end latency in the test instance: ~40 ms.
Between connections the source outputs digital silence. If PipeWire goes away (restart/crash), the GUI shows
"Virtual microphone unavailable — reconnecting to PipeWire…" (headless prints `VIRTUAL_MIC unavailable … (retrying)`)
and the source is re-created automatically when PipeWire is back.

The mic is created when the app starts (headless: before connecting), kept across disconnects/reconnects, and removed
on exit, Ctrl-C (SIGINT) and SIGTERM. Legacy null-sink/remap modules from older builds are unloaded at startup; an old
`module-pipe-source` is *not* unloaded automatically (that can crash PipeWire) — the app shows a hint to restart
PipeWire instead.

On a real PulseAudio server (no PipeWire) the app falls back to `module-pipe-source` fed through a FIFO in
`$XDG_RUNTIME_DIR` (unloading it is safe there). This fallback is not covered by the automated tests.

Only one running instance owns the virtual mic (lock file `$XDG_RUNTIME_DIR/vibe-phone-mic.lock`). A second
instance never touches it: headless prints `VIRTUAL_MIC error in-use: …` and streams without a virtual mic; a second
GUI window says so in its status card.

If an established connection is lost (not via Disconnect), the GUI shows "Reconnecting…" and retries the same phone
every 2 s until it is back or you press Disconnect. It gives up after 3 connections in a row drop within 3 s of
connecting. A first connect that fails just shows "Couldn't connect to …". When the phone sends `BYE replaced`
(another computer took over) the GUI shows "Another computer took over this phone." and does not reconnect; after
`BYE stopped` it keeps retrying. Headless mode prints `BYE {…}` / `DISCONNECTED <reason>` and exits.

## Develop

```sh
export CARGO_HOME=$PWD/../.cache/cargo   # keep crates inside the repo
cargo test                                # unit + integration tests (no display needed)
VPM_LONG_TESTS=1 cargo test --test pw_virtual_mic two_minutes   # 2-minute drift test
./build-appimage.sh                       # → ../dist/VibePhoneMic-x86_64.AppImage
python3 tools/fake_phone.py --freq 440    # fake phone (UDP discovery + TCP stream) for manual testing
```

Build needs `libpipewire-0.3` and `libpulse` development files and a C compiler (a small C shim, `src/pw_shim.c`,
wraps `pw_stream`; the AppImage links the system `libpipewire-0.3.so.0`).

`fake_phone.py` options: `--port`, `--freq`, `--rate`, `--channels`, `--name`, `--amplitude`, `--volume`,
`--muted`, `--mute-toggle SECS`, `--no-udp`, `--bind`.

**Audio tests never touch your audio daemon.** `pw_virtual_mic`, `pw_restart` and the virtual-mic part of `cli`
start a private PipeWire + pipewire-pulse + WirePlumber instance (`tools/pw-private.sh`: own runtime dir, no
ALSA/Bluetooth, a null sink as default output) and skip with a message if it can't start. In a minimal container
whose PipeWire lacks audio plugins, run `tools/fetch-pw-test-deps.sh` once (extracts the matching Arch packages into
`../.cache/pw-private/root`). For manual experiments: `eval "$(tools/pw-private.sh start /run/user/$UID/vpm-x)"`,
then `tools/pw-private.sh stop /run/user/$UID/vpm-x`.

GUI tests run the real UI code without a window; `gui_render` rasterizes it on the CPU (`src/render.rs`) and writes
PNGs to `target/tmp/gui-snapshots/`.

Layout: `src/protocol.rs` codec · `discovery.rs` UDP broadcast + localhost probe + /24 sweep · `client.rs` receiver
thread · `audio.rs` virtual mic host (PipeWire / PulseAudio fallback), WAV · `pwmic.rs` + `pw_shim.c` native PipeWire source + jitter buffer · `resample.rs` downmix/resample · `session.rs` client + outputs + state ·
`gui.rs` egui UI · `render.rs` offscreen rasterizer · `main.rs` CLI.
