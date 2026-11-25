# Vibe Phone Mic

Use an Android phone as a microphone for a Linux desktop, over Wi‑Fi or USB.

| Deliverable | Path |
|---|---|
| Android app (release, signed with a debug key) | `dist/VibePhoneMic.apk` |
| Linux desktop app | `dist/VibePhoneMic-x86_64.AppImage` |

## Quick start

1. Install `VibePhoneMic.apk` on the phone, open it, allow microphone access. It shows the phone's IP and port (48777).
2. On Linux run `./VibePhoneMic-x86_64.AppImage`. Pick the phone from the list (it scans the LAN), or type its IP, and press **Connect**.
3. In any app (Discord, OBS, browser…) choose **Vibe Phone Mic** as the microphone.

Mute, volume (0–200 %) and the microphone input (Settings) are controlled on the phone; the desktop only shows them.

**USB / direct:** `adb forward tcp:48777 tcp:48777`, then connect to `127.0.0.1` (the desktop scan finds it automatically).

Requirements on Linux: PipeWire (libpipewire-0.3 ≥ 0.3.49; the mic is a native PipeWire source), x86_64. Plain PulseAudio falls back to a pipe source (untested). Phone: Android 8.0+.

## Layout

- `android/` — Kotlin + Jetpack Compose app (server). See `android/README.md`.
- `desktop/` — Rust + egui app (client, virtual mic). See `desktop/README.md`.
- `sounds/` — button click sounds (vendored from joshwcomeau/button-sounds, MIT) and the script that cuts them.
- `e2e/` — end-to-end harness: real APK in a headless emulator whose mic is fed a 440 Hz tone, streaming to the real AppImage.
- `PLAN.md` — design, wire protocol, decisions and status.

## Tests

```sh
cd desktop && CARGO_HOME=../.cache/cargo cargo test          # 111 tests; audio tests use a private PipeWire instance
cd android && ./gradlew testDebugUnitTest                    # 123 tests (see android/README.md for env vars)
e2e/run_e2e.sh [--no-build] [--keep-emulator]                # full end-to-end, 30 checks
```

The e2e harness checks discovery (scan + UDP), real mic audio (440 Hz, no empty frames) in both the received WAV
and the `Vibe Phone Mic` PulseAudio source, mute/volume driven through the phone UI, client takeover (BYE),
microphone input selection, crashes, and saves screenshots of both apps to `e2e/out/`.
