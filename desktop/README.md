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

The virtual mic is `module-null-sink sink_name=vibe_phone_mic_sink` + `module-remap-source source_name=vibe_phone_mic`
(48 kHz mono; phone streams at other rates are converted by PulseAudio). It is created when the app starts (headless:
before connecting) and stays for the whole app lifetime, so apps keep it selected across disconnects and reconnects;
between connections it outputs silence. It is unloaded on exit, Ctrl-C (SIGINT) and SIGTERM. Modules left behind by
a crash are removed on the next start.

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
./build-appimage.sh                       # → ../dist/VibePhoneMic-x86_64.AppImage
python3 tools/fake_phone.py --freq 440    # fake phone (UDP discovery + TCP stream) for manual testing
```

`fake_phone.py` options: `--port`, `--freq`, `--rate`, `--channels`, `--name`, `--amplitude`, `--volume`,
`--muted`, `--mute-toggle SECS`, `--no-udp`, `--bind`.

Tests that need PulseAudio (`pulse_virtual_mic`, parts of `cli`/`gui_states`) skip themselves when no server is
running. GUI tests run the real UI code without a window; `gui_render` rasterizes it on the CPU
(`src/render.rs`) and writes PNGs to `target/tmp/gui-snapshots/`.

Layout: `src/protocol.rs` codec · `discovery.rs` UDP broadcast + localhost probe + /24 sweep · `client.rs` receiver
thread · `audio.rs` virtual mic, jitter-buffered Pulse playback, WAV · `session.rs` client + outputs + state ·
`gui.rs` egui UI · `render.rs` offscreen rasterizer · `main.rs` CLI.
