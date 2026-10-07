# Vibe Phone Mic — Plan

Use an Android phone as a microphone for a Linux desktop, over Wi‑Fi or a direct (USB/adb) connection.

- **Phone app** (`android/`) = **server**. Captures mic audio, applies mute/volume, serves it over TCP, answers LAN discovery. Shows its IP + port.
- **Desktop app** (`desktop/`) = **client**, shipped as an AppImage. Scans the LAN for phones (or manual IP entry), connects, receives audio and exposes it as a virtual microphone ("Vibe Phone Mic") in PulseAudio/PipeWire. It does *not* control volume/mute — the phone does.
- **E2E harness** (`e2e/`) runs the real APK in a local Android emulator, feeds a known tone into the emulator's mic, connects the real AppImage, and verifies the tone arrives (non-silent, correct frequency).

## Environment (audited 2026-10-06)

Present: JDK 17 (+25), Android SDK (platform-tools, build-tools 36, platform 36, cmdline-tools), Rust 1.97, Python 3.14, ffmpeg, pactl/parecord (PipeWire-pulse), GTK/libpulse dev files, /dev/kvm, network.
Installed by us (user space, no sudo): Android emulator + `system-images;android-35;google_apis;x86_64`, platform 35, Gradle (via wrapper), appimagetool (downloaded to `tools/`, gitignored).

## Home-dir rule (from user)
Nothing may be written to `~` outside this repo without the user's approval. Caches: `CARGO_HOME=.cache/cargo`, `GRADLE_USER_HOME=.cache/gradle`, `ANDROID_PREFS_ROOT=.cache/android-prefs` + `ANDROID_USER_HOME=.cache/android-prefs/.android` (repo-relative). Robolectric runs with user.home=android/.gradle/test-home. Approved exceptions: the sdkmanager install into `~/Android/Sdk` (emulator, android-35 image/platform, build-tools 35) and the AVD `vpm_e2e` under `~/.android/avd`.

## Wire protocol v1 (both apps MUST follow exactly)

Port: **48777** for both TCP (stream) and UDP (discovery). All integers **big-endian**.

### Discovery (UDP 48777)
- Desktop broadcasts ASCII `VPMIC_DISCOVER_V1` (no newline) to 255.255.255.255:48777 and to each interface's directed broadcast address.
- Phone replies (unicast to sender addr/port) with UTF‑8 JSON:
  `{"service":"vibe-phone-mic","version":1,"name":"<device name>","port":48777}`
- Desktop additionally (a) TCP-probes `127.0.0.1:48777` (USB via `adb forward`), and (b) does a TCP probe sweep of each local IPv4 /24 (short timeout, bounded concurrency) as a fallback when broadcast is blocked.

### TCP stream (TCP 48777)
1. Client connects and sends an 8‑byte handshake: ASCII `VPMC`, u8 version=`1`, u8 mode (`0`=stream, `1`=probe), 2 zero bytes.
2. Server validates; on bad handshake closes the socket.
3. Server sends frames: `u8 type | u32 payloadLen | payload`.
   - `0x01 HELLO` — JSON `{"name":"Pixel 7","sampleRate":48000,"channels":1,"format":"s16le","frameMs":10,"version":1}`. Always first.
   - `0x02 AUDIO` — `u32 seq | u64 captureTimeMicros | PCM s16le interleaved samples`. Sent continuously while streaming, **including while muted** (then PCM is zeros).
   - `0x03 STATUS` — JSON `{"muted":false,"volume":100,"input":"Built-in microphone"}` (volume = percent gain 0–200). Sent right after HELLO and whenever it changes.
   - `0x04 BYE` — JSON `{"reason":"replaced"|"stopped"}`. Sent by the phone right before it closes a stream client on purpose (another client took over, or the user stopped the server). Desktop must NOT auto-reconnect after `replaced`; shows "Another computer took over this phone". After `stopped` it may keep retrying quietly. Unknown frame types must be ignored (forward compat).
4. Probe mode: server sends HELLO then closes. Used by scan.
5. One streaming client at a time; a new stream client replaces the old one (old socket closed).
6. Desktop must honor HELLO's sampleRate/channels (48000 mono preferred; phone may fall back to 44100).
7. Max payload 1 MiB; larger = protocol error.

## Desktop app (`desktop/`, Rust)
- Crate `vibe-phone-mic`: `lib` (protocol codec, discovery, client/receiver thread, audio output trait + PulseAudio impl, WAV writer) and `bin` (eframe/egui GUI + headless CLI).
- Virtual mic: a native PipeWire stream that IS an `Audio/Source` node (`node.name=vibe_phone_mic`, description "Vibe Phone Mic", `node.virtual=true`, `priority.session=1`, 48 kHz mono S16) created with libpipewire (C shim over `pw_stream`). NO PulseAudio modules on PipeWire: a remap-source can fall back to the default sink monitor (system audio leaked into the mic), and unloading `module-pipe-source` crashes PipeWire 1.6.8 (pipe-tunnel use-after-free). Desktop downmixes/resamples to 48 kHz mono; RT callback pulls from a 40 ms jitter buffer (silence on underrun). Created at app start, kept across disconnects, removed on exit; vanishes automatically if the app dies; re-created when PipeWire restarts. Single owner via lock file. Legacy null-sink/remap modules are unloaded at startup; legacy pipe-sources are left alone with a "restart PipeWire" hint. Real PulseAudio (no PipeWire) falls back to module-pipe-source. All audio tests run against a private PipeWire instance (desktop/tools/pw-private.sh), never the user's daemon.
- CLI (used by tests/e2e):
  - `--scan [--timeout-ms N]` → prints JSON array of found servers.
  - `--headless --connect HOST[:PORT] [--wav out.wav] [--duration SECS] [--no-virtual-mic]` → streams; logs HELLO/STATUS; writes received PCM to WAV.
- GUI: discovered phone list + Rescan, manual host/port entry, Connect/Disconnect, connection state, phone mute/volume (read-only, from STATUS), input level meter, hint "select *Vibe Phone Mic* as your input".
- Tests: protocol unit tests, a fake phone server in Rust integration tests (sine wave → client → verify), discovery test on loopback, GUI snapshot/smoke test if feasible.
- Packaging: `desktop/build-appimage.sh` → `dist/VibePhoneMic-x86_64.AppImage` (manual AppDir + appimagetool; links system libpulse/GL dynamically).

## Android app (`android/`, Kotlin + Jetpack Compose)
- Package `com.vibephonemic`, minSdk 26, compile/target 35+. Gradle wrapper committed.
- Foreground service (type `microphone`) owns: AudioRecord (48 kHz mono 16‑bit, 10 ms frames), TCP server, UDP discovery responder. Server auto-starts when the app opens and RECORD_AUDIO is granted.
- Gain/mute applied on phone (gain 0–200 %, clipped). Mute → zero PCM frames + STATUS.
- Main screen: IP address(es) + port big and clear, connection status (client IP), mute toggle, volume slider, level meter, start/stop.
- Settings screen: microphone input selection (AudioManager input devices → `setPreferredDevice`, plus "Default"), persisted.
- Pure-Kotlin protocol/gain/discovery code in testable classes with JVM unit tests (incl. a socket-level test with a Kotlin test client).

## Style (both apps)
Clean and readable. White / dark-gray surfaces and text. **Orange→red gradient (#FF8A00 → #E52E2E)** as the single pop color, used sparingly (primary action / connected state / logo). No other gradients or accent colors. Support dark theme (dark gray) where natural.

## Emulator notes (verified)
- Launch via `e2e/start_emulator.sh` / `e2e/stop_emulator.sh` (headless, screen may be off).
- `TCMALLOC_SKIP_SBRK=true` avoids SwiftShader execheap SELinux segfault.
- Must use `-qt-hide-window` (not `-no-window`; headless binary lacks pulse audio) with QT_QPA_PLATFORM=offscreen, DISPLAY/WAYLAND unset.
- HOME/XDG_* redirected to `.cache/android/home`; libxkbfile symlinked from host into `.cache/android/hostlibs` (we run in an Arch toolbox).
- Emulator mic HAL wedges (pcm_readi I/O error → silence) under guest UI load. Harness: `device_config put privacy camera_mic_icons_enabled false`, animation scales 0, stream with activity backgrounded; check logcat `pcm_readi failed` count==0, retry otherwise. `adb emu` needs HOME=.cache/android/home ANDROID_USER_HOME=.cache/android.
- Guest mic verified: 440 Hz tone via PULSE_SOURCE=vpm_e2e_src.monitor + `adb emu avd hostmicon`. Analyzer: `e2e/analyze_tone.py`.

## E2E (`e2e/run_e2e.sh`)
1. Build APK + AppImage.
2. Create isolated pulse null sink `vpm_e2e_src`, play a 440 Hz tone into it.
3. Boot AVD `vpm_e2e` with `PULSE_SOURCE=vpm_e2e_src.monitor` and host mic enabled; install APK, grant perms, launch.
4. `adb forward tcp:48777 tcp:48777`; AppImage `--scan` must find 127.0.0.1; AppImage `--headless --connect 127.0.0.1 --wav out.wav --duration 6`.
5. Analyze WAV: RMS above threshold, dominant frequency ≈ 440 Hz; also record from the `vibe_phone_mic` pulse source and verify the same.
6. Toggle mute via UI automation → silence + STATUS muted; change volume → level changes.
7. Screenshots of both apps saved to `e2e/out/` for visual review.

## Test coverage rule (from user)
Each app must have tests for all reasonable states, including settings, and those suites must be run against the real implementation before a feature is accepted. Android: server states, mute/volume extremes, permission states, every settings state (default/specific/vanished device/persistence), main-screen UI states, debug tone hook. Desktop: all client/connection states incl. errors, sample rates, scan result kinds, manual-entry validation, virtual-mic lifecycle, GUI view per state.

## Virtual mic history (important)
1. null-sink + remap-source: LEAKED SYSTEM AUDIO. pipewire-pulse remap's capture stream falls back to the default sink monitor when its master isn't resolvable at link time.
2. module-pipe-source: no leak, but PipeWire 1.6.8 SEGFAULTS on unload (pipe-tunnel on_timeout vs impl_destroy race) -> crashes the user's audio server. Do not ship.
3. Native PipeWire stream with media.class=Audio/Source (SHIPPED): no modules, no fallback, auto-cleanup on crash; lock-free SPSC ring jitter buffer (~40 ms), node.latency=1024/48000, fades on trims/underruns, no late-attach click. pipe-source only as a fallback for real PulseAudio.
Rule: crash-prone audio experiments and module-loading tests run against a PRIVATE PipeWire instance, never the user's live session. e2e must play a distractor to the default sink and assert it doesn't leak into the mic.

## Deferred / known gaps
- Flatpak Discord re-test of the native source by the user (host checks pass: no leak, no clicks, cleanup OK).
- Pure-PulseAudio fallback (pipe-source) untested (no PulseAudio server available).
- WirePlumber may make "Vibe Phone Mic" the default input if it is the only source (priority.session=1 otherwise).
- 2-minute drift test tolerates a click only when PipeWire reports a simultaneous xrun (no RT scheduling in the toolbox).
- desktop/README.md jitter-buffer details may lag the lock-free ring implementation.
- AppImage built against Arch/Fedora-new glibc; older distros untested.

## Process
- Implementation agents per app; independent verifier agent re-tests each feature before it's accepted.
- Status log below is updated as work completes.

## Status
- [x] Tool audit / install (appimagetool in tools/, SDK pkgs approved)
- [x] Desktop app implemented (63 tests); follow-up: persistent virtual mic, auto-reconnect, disabled-button style
- [x] Desktop verified by second agent; round-2 fixes re-verified PASS (84 tests)
- [x] Android app implemented (79 tests)
- [x] Android verified by second agent (all functional PASS on emulator → fix round 2: BYE, STOP crash, layout, AudioRecord watchdog, MicService tests)
- [x] E2E harness green in emulator (27/27 2026-10-06; 30/30 2026-10-07 with native PipeWire mic, AppImage run on host, system-audio leak check)
- [x] Final deliverables in `dist/` (VibePhoneMic.apk, VibePhoneMic-x86_64.AppImage)
