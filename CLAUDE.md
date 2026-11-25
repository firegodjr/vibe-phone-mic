# Vibe Phone Mic: notes for agents

The overview, build steps and test commands are in `README.md`, `android/README.md` and `desktop/README.md`.
Keep build state inside the repo. The env vars for this are in those READMEs: `CARGO_HOME`, `GRADLE_USER_HOME`,
`ANDROID_PREFS_ROOT` and `ANDROID_USER_HOME` all point under `.cache/`. Audio tests use a private PipeWire
instance (`desktop/tools/pw-private.sh`) and never the user's daemon.

## Button sounds

The clips come from joshwcomeau/button-sounds (MIT). The general how-to is the user-level `button-sounds` skill.
The choices below are specific to this app.

- **Vendored, not a submodule.** `sounds/extract.py <checkout>` cuts the clips. The `SOUNDS` dict lists the
  selection and which app gets each sound. The license and upstream commit are in `sounds/`. Outputs are
  committed: `android/app/src/main/res/raw/sfx_*.ogg` and `desktop/assets/sfx/*.wav`.
- **Mapping:**

  | Sound | Gain | Android | Desktop |
  |---|---|---|---|
  | `kvm-switch` | 0.9 (quiet recording) | server Start/Stop, Grant access | Connect / Disconnect |
  | `carson-microflip` | 0.25 | settings, back, volume ±, input rows | Rescan, phone rows |
  | `halo-space` | 0.45 | mute toggle | (the desktop app has no mute button) |

- **The phone's mic is live, so clicks must never be streamed** (a user requirement):
  - The mute toggle (`ButtonSound.gated = true`) always plays. Before its clip, `ButtonSounds` closes
    `AppState.soundGate` (`core/SilenceGate.kt`) for `GATE_HOLD_MS` = 500 ms. While the gate is closed,
    `MicServer` sends zero frames.
  - The gate doesn't change the reported mute or the UI. Muting goes silent on press. Unmuting shows "live" at
    once but stays silent until the release click has played. The button color must change immediately.
  - Every other Android sound plays only while the user's mute setting is on (`ButtonSounds.shouldPlay`). Don't
    gate them: that would punch holes in live audio.
- **Android wiring:** `ui/ButtonSounds.kt` holds the `ButtonSound` enum, the `SoundPool` player,
  `LocalSoundPlayer` and `rememberSoundInteraction(sound)`. Pass the last one as a component's
  `interactionSource`. `MainActivity` creates the player with the gate and a mute lambda, and releases it in
  `onDestroy`. New clickables get a sound by passing `interactionSource = rememberSoundInteraction(...)`. For
  `toggleable`/`selectable`, also pass `indication = LocalIndication.current`.
- **Desktop wiring:** `src/sfx.rs` plays the clips through libpulse-simple on a background thread. It plays to the
  default sink, never the app's virtual mic. The stream closes after 4 s idle. Call
  `self.sfx.button(ui, &resp, Sound::…)` after adding a widget. Sounds are on only in the real GUI
  (`GuiOptions.sounds`; `--no-sounds` turns them off). Tests, headless mode and `--screenshot` stay silent.
- **Tests:**
  - Android: `ButtonSoundsTest` (UI mapping and the gate/mute policy), `SilenceGateTest`, and
    `MicServerTest.closedSilenceGateStreamsZerosWithoutReportingMute`.
  - Desktop: `tests/gui_sfx.rs` (real pointer events), `tests/pw_sfx.rs` (real playback on the private PipeWire
    instance), and the unit tests in `src/sfx.rs`.
