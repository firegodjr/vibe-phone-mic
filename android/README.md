# Vibe Phone Mic: Android app

This app turns an Android phone (API 26+) into a network microphone. It acts as the **server** for wire protocol v1, which is described in `../PLAN.md`:

- **TCP 48777** streams 48 kHz mono s16le audio in 10 ms frames. It falls back to 44.1 kHz if 48 kHz is unavailable, and HELLO reports the rate in use.
- **UDP 48777** answers `VPMIC_DISCOVER_V1` discovery requests.
- Mute and volume (0–200 %, clipped) are applied on the phone. While muted, frames keep flowing but contain zeros.
- Before the phone deliberately closes a stream client, it sends `0x04 BYE`:
  - `{"reason":"replaced"}` when another client takes over.
  - `{"reason":"stopped"}` when the user stops the server.
- A capture watchdog reopens `AudioRecord` (bounded retries, logged) when it delivers about 1 s of exact digital zeros while unmuted, or when reads keep failing. If reads still fail after the retries, the app switches to silence and reports "Microphone unavailable".

## Build

The build expects JDK 17 and an Android SDK with platform 36 and build-tools 36.

```sh
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk
export ANDROID_HOME=$HOME/Android/Sdk
# keep Gradle/AGP state inside the repo:
export GRADLE_USER_HOME=$PWD/../.cache/gradle
export ANDROID_PREFS_ROOT=$PWD/../.cache/android-prefs            # AGP analytics would otherwise write ~/.android
export ANDROID_USER_HOME=$PWD/../.cache/android-prefs/.android    # must equal $ANDROID_PREFS_ROOT/.android (holds debug.keystore)

./gradlew assembleDebug        # app/build/outputs/apk/debug/app-debug.apk
./gradlew assembleRelease      # app/build/outputs/apk/release/app-release.apk (R8, signed with the debug key)
./gradlew testDebugUnitTest    # JVM + Robolectric tests
```

Unit tests point the test JVM's `user.home` at `android/.gradle/test-home`. This keeps Robolectric's `android-all` download out of `~/.m2`.
`ScreenshotTest` renders the main screen with Robolectric native graphics, checks pixel colours, and writes PNGs to `app/build/screenshots/`.

## Use

1. Install the app, open it and grant microphone access. The server starts automatically and runs as a microphone foreground service.
2. The main screen shows the phone's IPv4 address(es) and port **48777**. Connect the desktop app to one of them.
3. **USB:** run `adb forward tcp:48777 tcp:48777` on the computer, then connect to `127.0.0.1`.
4. In **Settings** (the gear icon), you can choose which microphone input to use. The choice is saved.

Buttons click. A press plays a random "down" sample and the release plays a random "up" sample (see `../sounds/`). The up sample starts at least 50 ms after the down sample. The sounds play on the sonification stream, so the system volume controls them. The clicks are never streamed:
- **Mute button:** before its clip plays, `ButtonSounds` closes `AppState.soundGate` (`core/SilenceGate.kt`) for 200 ms (plus any release delay), and while the gate is closed `MicServer` sends zero frames. The reported mute state doesn't change and the UI updates immediately. Pressing Mute silences the stream at once, and unmuting keeps it silent until the release click has played.
- **All other buttons:** they click only while the mic is muted. While it's live they make no sound.

## Code layout

| Path | Contents |
|---|---|
| `core/` (pure Kotlin, no Android imports) | `Protocol.kt` (handshake, frame codec, discovery reply), `AudioMath.kt` (gain, mute, clipping, meter), `AudioSource.kt` (interface, sine, silence), `MicServer.kt` (TCP server and capture loop), `DiscoveryResponder.kt` (UDP), `Json.kt` |
| `MicService.kt` | The foreground service. It wires `MicServer`, `DiscoveryResponder`, `AudioRecordSource`, the multicast/Wi‑Fi/wake locks and the notification together. |
| `AudioRecordSource.kt` | Captures with `AudioRecord`, using `setPreferredDevice`. |
| `AppState.kt` | Settings persisted in SharedPreferences, plus live server state shared through StateFlows. |
| `ui/ButtonSounds.kt` | Click sounds: `SoundPool` player, `LocalSoundPlayer`, and `rememberSoundInteraction(sound)`. Pass the result as a component's `interactionSource`. |
| `ui/` | Compose screens and the neutral theme. The only accent is the orange→red gradient. |

## Automation hooks

Compose test tags are exposed as resource-ids (`testTagsAsResourceId`), so `uiautomator dump` can see them:

`server_toggle` is the compact Start/Stop button in the status card.

`mute_button` (a toggle: `checked` = muted, content-desc "Mute microphone"), `mute_state`, `volume_slider` (content-desc "Volume", 0–200), `volume_up` and `volume_down` (±10 %), `volume_text`, `level_meter`, `ip_text` (one per address), `port_text`, `usb_hint`, `connection_status`, `audio_info`, `error_text`, `server_toggle`, `settings_button`, `permission_card`, `grant_permission`, `no_network_text`, `back_button`, `input_option_default`, `input_option_<deviceId>`.

**Debug builds only:** this command replaces the microphone with a generated sine:

```sh
adb shell am start -n com.vibephonemic/.MainActivity --ez test_tone true --ei test_tone_hz 440
adb shell am start -n com.vibephonemic/.MainActivity --ez test_tone false   # back to the mic
```

In release builds the hook is guarded by `BuildConfig.DEBUG`, and R8 strips it out.
