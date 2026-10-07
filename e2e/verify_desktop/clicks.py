#!/usr/bin/env python3
"""Find discontinuities in a raw s16le 48k mono recording of a pure tone at F Hz.
Usage: clicks.py FILE F [threshold]  -> prints positions (s) of residual spikes."""
import sys, struct, math
f, F = sys.argv[1], float(sys.argv[2]); th = float(sys.argv[3]) if len(sys.argv) > 3 else 300
d = open(f, 'rb').read(); x = struct.unpack('<%dh' % (len(d) // 2), d)
k = 2 - 2 * math.cos(2 * math.pi * F / 48000)
spikes = [n for n in range(2, len(x)) if abs(x[n] - 2 * x[n-1] + x[n-2] + k * x[n-1]) > th]
groups = []
for n in spikes:
    if not groups or n - groups[-1][-1] > 480: groups.append([n])
    else: groups[-1].append(n)
print(f"{f}: {len(x)/48000:.2f}s, discontinuities={len(groups)} at s=", [round(g[0]/48000, 3) for g in groups][:30])
