#!/usr/bin/env python3
"""Cut the button-sounds sprite WAVs into one file per sample for the Android and desktop apps.

Usage: python3 -I sounds/extract.py /path/to/button-sounds-checkout

Upstream (github.com/joshwcomeau/button-sounds, MIT) ships each sound as one 48 kHz stereo sprite:
1000 ms of silence, then for sample n (1-based) a "down" (press) clip at n*1000 ms (190 ms long)
and an "up" (release) clip at n*1000+200 ms (790 ms long). The sample count per sound comes from
package/src/core/sprite-data.ts. Needs ffmpeg on PATH. Outputs are committed, so this only has to
be re-run when the selection below changes.
"""
import array
import os
import re
import subprocess
import sys

# Sounds the apps use, and which app gets them.
SOUNDS = {
    "kvm-switch": ("android", "desktop"),        # small buttons: server start/stop, connect/disconnect
    "carson-microflip": ("android", "desktop"),  # tiny buttons: settings, back, +/-, list rows
    "halo-space": ("android",),                  # the big mute toggle (Android only)
}
DOWN_MS, UP_MS = 190, 790
RATE = 48000
SILENCE = 0.001   # trailing samples below this (≈ -60 dBFS) are trimmed
FADE_MS = 5       # short fade-out so trimmed tails never click

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ANDROID_OUT = os.path.join(ROOT, "android/app/src/main/res/raw")
DESKTOP_OUT = os.path.join(ROOT, "desktop/assets/sfx")


def sample_counts(repo):
    text = open(os.path.join(repo, "package/src/core/sprite-data.ts"), encoding="utf-8").read()
    return {m[0] or m[1]: int(m[2]) for m in re.findall(r"(?:'([\w-]+)'|(\w+)):\s*getSubsetOfData\((\d+)\)", text)}


def decode(path):
    """Whole file as interleaved stereo float32 at 48 kHz."""
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-f", "f32le", "-ac", "2", "-ar", str(RATE), "-"],
        check=True, capture_output=True,
    ).stdout
    return array.array("f", raw)


def cut(pcm, start_ms, dur_ms):
    a = start_ms * RATE // 1000 * 2
    b = a + dur_ms * RATE // 1000 * 2
    clip = list(pcm[a:b])
    end = len(clip)
    while end > 2 and abs(clip[end - 1]) < SILENCE and abs(clip[end - 2]) < SILENCE:
        end -= 2
    clip = clip[:end]
    fade = min(FADE_MS * RATE // 1000, len(clip) // 2)
    for i in range(fade):
        g = i / fade
        clip[len(clip) - 2 * i - 1] *= g
        clip[len(clip) - 2 * i - 2] *= g
    return clip


def encode(clip, out, args):
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "f32le", "-ac", "2", "-ar", str(RATE), "-i", "-", *args, out],
        input=array.array("f", clip).tobytes(), check=True,
    )


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    repo = sys.argv[1]
    counts = sample_counts(repo)
    os.makedirs(ANDROID_OUT, exist_ok=True)
    os.makedirs(DESKTOP_OUT, exist_ok=True)
    for name, targets in SOUNDS.items():
        pcm = decode(os.path.join(repo, "package/audio", name + ".wav"))
        n = counts[name]
        for i in range(1, n + 1):
            for kind, start, dur in (("down", i * 1000, DOWN_MS), ("up", i * 1000 + 200, UP_MS)):
                clip = cut(pcm, start, dur)
                peak = max(abs(x) for x in clip)
                base = f"{name.replace('-', '_')}_{kind}{i}"
                if "android" in targets:
                    encode(clip, os.path.join(ANDROID_OUT, f"sfx_{base}.ogg"), ["-c:a", "libvorbis", "-q:a", "5"])
                if "desktop" in targets:
                    encode(clip, os.path.join(DESKTOP_OUT, f"{base}.wav"), ["-c:a", "pcm_s16le"])
                print(f"{base:28s} {len(clip) // 2 * 1000 // RATE:4d} ms  peak {peak:.3f}")


if __name__ == "__main__":
    main()
