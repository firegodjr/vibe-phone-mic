#!/usr/bin/env python3
"""Goertzel-based leak checks on raw s16le mono 48 kHz captures.

  leak_check.py played FILE DIST_HZ          -> exit 0 if DIST_HZ is clearly present (distractor really played)
  leak_check.py clean  FILE DIST_HZ TONE_HZ  -> exit 0 if TONE_HZ is present and DIST_HZ is >= 60 dB below it
"""
import array, math, sys

def power(a, f, rate=48000):
    k = 2 * math.cos(2 * math.pi * f / rate); s1 = s2 = 0.0
    for x in a:
        s0 = x + k * s1 - s2; s2 = s1; s1 = s0
    return (s1 * s1 + s2 * s2 - k * s1 * s2) / max(1, len(a)) ** 2

mode, path = sys.argv[1], sys.argv[2]
a = array.array('h', open(path, 'rb').read())
if len(a) < 48000:
    print(f"{path}: only {len(a)} samples"); sys.exit(1)
dist = power(a, float(sys.argv[3]))
if mode == "played":
    # amplitude of a full-scale-normalised sine ~ 2*sqrt(P)/32768
    amp = 2 * math.sqrt(dist) / 32768
    print(f"distractor amplitude on speakers: {amp:.4f}")
    sys.exit(0 if amp > 0.0005 else 1)
tone = power(a, float(sys.argv[4]))
ratio_db = 10 * math.log10((dist + 1e-30) / (tone + 1e-30))
print(f"virtual mic: distractor/tone = {ratio_db:.1f} dB")
sys.exit(0 if tone > 0 and ratio_db < -60 else 1)
