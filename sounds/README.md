# UI sounds

Button click sounds come from [joshwcomeau/button-sounds](https://github.com/joshwcomeau/button-sounds)
(MIT, © Josh Comeau, see `LICENSE-button-sounds`). Only the clips the apps use are vendored. The upstream repo
is not a submodule.

| Sound | Used for | Gain |
|---|---|---|
| `kvm-switch` | small buttons: Android server Start/Stop and Grant access; desktop Connect/Disconnect | 0.9 (quiet recording) |
| `carson-microflip` | tiny buttons: Android settings, back, volume ±, input rows; desktop Rescan and phone rows | 0.25 |
| `halo-space` | Android mute toggle | 0.45 |

Each upstream sound is a single sprite file that holds 4–8 recordings. `extract.py` cuts it into one file per
recording: `down<n>` is the press and `up<n>` is the release. The apps play a random down sample when the finger or
pointer goes down, and a random up sample on release. They never play the same sample twice in a row.

The Android app never streams its own clicks. The mute button's clip first closes a 500 ms silence gate on the
outgoing mic stream (`core/SilenceGate.kt`). All other Android buttons click only while the mic is muted.

Outputs (committed):
- `android/app/src/main/res/raw/sfx_<sound>_{down,up}<n>.ogg` (Vorbis), played through `SoundPool` (`ui/ButtonSounds.kt`)
- `desktop/assets/sfx/<sound>_{down,up}<n>.wav` (48 kHz stereo s16), embedded with `include_bytes!` (`src/sfx.rs`)

To regenerate after changing the selection in `extract.py` (needs ffmpeg):

```sh
git clone --depth 1 https://github.com/joshwcomeau/button-sounds /some/scratch/dir
python3 -I sounds/extract.py /some/scratch/dir
```

Vendored from upstream commit `e0fd81a`.
