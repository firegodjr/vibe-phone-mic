#!/usr/bin/env python3
"""Stdlib-only tone analyzer. Usage: analyze_tone.py FILE [--rate 48000] [--channels 1] [--expect 440]
FILE may be a .wav (header parsed) or raw s16le. Prints RMS (dBFS) and dominant frequency (coarse-to-fine Goertzel, 50-8000 Hz, +-2 Hz)."""
import argparse, math, struct, sys, wave

def load(path, rate, ch):
    if path.endswith(".wav"):
        with wave.open(path, "rb") as w:
            rate, ch, sw = w.getframerate(), w.getnchannels(), w.getsampwidth()
            assert sw == 2, "only 16-bit wav supported"
            data = w.readframes(w.getnframes())
    else:
        data = open(path, "rb").read()
    n = len(data) // 2
    s = struct.unpack("<%dh" % n, data[: n * 2])
    if ch > 1:
        s = s[::ch]
    return [x / 32768.0 for x in s], rate

def goertzel(x, rate, f):
    w = 2 * math.pi * f / rate; c = 2 * math.cos(w); s1 = s2 = 0.0
    for v in x:
        s0 = v + c * s1 - s2; s2 = s1; s1 = s0
    return s1 * s1 + s2 * s2 - c * s1 * s2

def hann(x):
    n = len(x)
    if n < 2:
        return list(x)
    return [v * (0.5 - 0.5 * math.cos(2 * math.pi * i / (n - 1))) for i, v in enumerate(x)]

def best_in(x, rate, lo, hi, step):
    best_f, best_p, f = lo, -1.0, lo
    while f <= hi + 1e-9:
        p = goertzel(x, rate, f)
        if p > best_p:
            best_f, best_p = f, p
        f += step
    return best_f, best_p

def dominant(x, rate):
    """Coarse-to-fine Goertzel search, 50..8000 Hz (below Nyquist), within +-2 Hz.
    Short window (wide lobe) on a 10 Hz grid so no tone falls between grid points,
    then longer windows narrow the search, then a parabolic fit."""
    full = x[-rate:]  # last <= 1 s
    hi = min(8000.0, rate / 2 - 50)
    f1, p1 = best_in(hann(full[-(rate // 40):]), rate, 50.0, hi, 10.0)
    if p1 <= 0:
        return 0.0
    f2, _ = best_in(hann(full[-(rate // 5):]), rate, max(1.0, f1 - 40), f1 + 40, 2.0)
    longw = hann(full); step = 0.25
    f3, p3 = best_in(longw, rate, max(1.0, f2 - 4), f2 + 4, step)
    pl, pr = goertzel(longw, rate, f3 - step), goertzel(longw, rate, f3 + step)
    d = pl - 2 * p3 + pr
    delta = max(-0.5, min(0.5, 0.5 * (pl - pr) / d)) if d != 0 else 0.0
    return f3 + delta * step

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file"); ap.add_argument("--rate", type=int, default=48000)
    ap.add_argument("--channels", type=int, default=1); ap.add_argument("--expect", type=float, default=440.0)
    ap.add_argument("--tol", type=float, default=15.0); ap.add_argument("--min-dbfs", type=float, default=-50.0)
    a = ap.parse_args()
    x, rate = load(a.file, a.rate, a.channels)
    skip = rate // 2  # drop first 0.5 s (startup)
    x = x[skip:] if len(x) > 2 * skip else x
    x = x[: rate * 2]  # analyze up to 2 s
    mean = sum(x) / len(x); x = [v - mean for v in x]
    rms = math.sqrt(sum(v * v for v in x) / len(x)); db = 20 * math.log10(rms) if rms > 0 else -999
    fine = dominant(x, rate)
    ok = abs(fine - a.expect) <= a.tol and db >= a.min_dbfs
    print("samples=%d rate=%d rms=%.4f (%.1f dBFS) dominant=%.1f Hz expect=%.0f -> %s" % (len(x), rate, rms, db, fine, a.expect, "PASS" if ok else "FAIL"))
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
