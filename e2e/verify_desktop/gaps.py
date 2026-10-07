#!/usr/bin/env python3
"""Report length and silent / discontinuous 10 ms blocks in a s16le mono file (raw or wav)."""
import sys, struct, wave, math
p = sys.argv[1]; rate = 48000
if p.endswith(".wav"):
    w = wave.open(p); rate = w.getframerate(); ch = w.getnchannels(); d = w.readframes(w.getnframes())
else:
    ch = 1; d = open(p, "rb").read()
s = struct.unpack("<%dh" % (len(d)//2), d[:len(d)//2*2])[::ch]
blk = rate // 100
rms = [math.sqrt(sum(x*x for x in s[i:i+blk]) / blk) / 32768 for i in range(0, len(s)-blk+1, blk)]
silent = [i for i, r in enumerate(rms) if r < 0.01]
first_loud = next((i for i, r in enumerate(rms) if r >= 0.01), None)
last_loud = max((i for i, r in enumerate(rms) if r >= 0.01), default=None)
inner = [i for i in silent if first_loud is not None and first_loud < i < last_loud]
print(f"{p}: {len(s)/rate:.2f}s blocks={len(rms)} silent_blocks={len(silent)} first_loud={first_loud} last_loud={last_loud} silent_inside_loud_span={len(inner)} {inner[:20]}")
