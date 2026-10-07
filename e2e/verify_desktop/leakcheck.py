#!/usr/bin/env python3
"""Record a PulseAudio/PipeWire source safely and report tone content.

Usage:
  leakcheck.py rec SOURCE SECS OUT.raw [--tone F]   record SOURCE (asserting it exists and that
                                                    the recording stream is really attached to it)
  leakcheck.py distract SECS [--freq 1000] [--amp 0.05]  play a quiet tone on the DEFAULT sink
  leakcheck.py links                                 print pw-link lines that FEED vibe_phone_mic
Prints JSON. Amplitudes are linear (1.0 = full scale sine).
"""
import json, math, struct, subprocess, sys, time

def sh(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout

def source_index(name):
    for line in sh("pactl", "list", "short", "sources").splitlines():
        f = line.split("\t")
        if len(f) > 1 and f[1] == name:
            return f[0]
    return None

def tone_amp(x, rate, f):
    """Amplitude of a sinusoid at f (Hann-windowed Goertzel, coherent gain corrected)."""
    n = len(x)
    if n == 0:
        return 0.0
    w = 2 * math.pi * f / rate; c = 2 * math.cos(w); s1 = s2 = 0.0
    for i, v in enumerate(x):
        h = 0.5 - 0.5 * math.cos(2 * math.pi * i / (n - 1))
        s0 = v * h + c * s1 - s2; s2 = s1; s1 = s0
    p = s1 * s1 + s2 * s2 - c * s1 * s2
    return 2 * math.sqrt(max(p, 0)) / (n * 0.5)

def feeders():
    """Links whose INPUT side is a vibe_phone_mic port (something feeding the mic)."""
    out = sh("pw-link", "-l")
    bad = []
    cur = None
    for line in out.splitlines():
        if not line.startswith(" "):
            cur = line.strip()
            continue
        s = line.strip()
        # pw-link -l prints "port\n  |-> peer" (output feeding peer) and "port\n  |<- peer" (peer feeding port)
        if s.startswith("|<-") and cur and "vibe_phone_mic" in cur:
            bad.append(f"{s[3:].strip()} -> {cur}")
        if s.startswith("|->") and "vibe_phone_mic" in s and cur and "vibe_phone_mic" not in cur:
            bad.append(f"{cur} -> {s[3:].strip()}")
    # Also: does vibe_phone_mic own any input ports at all?
    inputs = [l for l in sh("pw-link", "-i").splitlines() if "vibe_phone_mic" in l]
    return bad, inputs

def rec(source, secs, out, tones):
    if source != "@DEFAULT_MONITOR@":
        idx = source_index(source)
        if idx is None:
            print(json.dumps({"error": f"source {source} does not exist - refusing to record (would fall back)"}))
            sys.exit(3)
    p = subprocess.Popen(["parecord", "--latency-msec=20", "-d", source, "--raw", "--format=s16le",
                          "--rate=48000", "--channels=1", out])
    time.sleep(0.4)
    attached = None
    if source != "@DEFAULT_MONITOR@":
        for line in sh("pactl", "list", "short", "source-outputs").splitlines():
            f = line.split("\t")
            if len(f) > 1 and f[1] == idx:
                attached = True
        if not attached:
            p.kill(); p.wait()
            print(json.dumps({"error": f"recording stream not attached to {source} (idx {idx})",
                              "source_outputs": sh("pactl", "list", "short", "source-outputs")}))
            sys.exit(4)
    time.sleep(max(0, secs - 0.4))
    still = source == "@DEFAULT_MONITOR@" or source_index(source) == idx
    p.terminate(); p.wait()
    d = open(out, "rb").read()
    s = struct.unpack("<%dh" % (len(d) // 2), d[: len(d) // 2 * 2])
    x = [v / 32768 for v in s]
    tail = x[len(x) // 4:] if len(x) > 9600 else x  # skip start
    res = {"source": source, "secs": round(len(x) / 48000, 2), "source_still_same": still,
           "peak": max((abs(v) for v in x), default=0), "rms": math.sqrt(sum(v * v for v in tail) / max(1, len(tail))),
           "nonzero_samples": sum(1 for v in s if v != 0)}
    for f in tones:
        res[f"amp_{f:g}"] = tone_amp(tail[:48000 * 3], 48000, f)
    bad, inputs = feeders()
    res["feeding_links"] = bad
    res["mic_input_ports"] = inputs
    print(json.dumps(res))

def distract(secs, freq, amp):
    n = int(48000 * secs)
    data = b"".join(struct.pack("<h", int(math.sin(2 * math.pi * freq * i / 48000) * amp * 32767)) for i in range(n))
    p = subprocess.Popen(["pacat", "--playback", "--format=s16le", "--rate=48000", "--channels=1",
                          "--latency-msec=50"], stdin=subprocess.PIPE)
    p.stdin.write(data); p.stdin.close(); p.wait()

if __name__ == "__main__":
    a = sys.argv[1:]
    def opt(name, default):
        return float(a[a.index(name) + 1]) if name in a else default
    if a[0] == "rec":
        tones = [1000.0, 1100.0] + ([opt("--tone", 0)] if "--tone" in a else [])
        rec(a[1], float(a[2]), a[3], tones)
    elif a[0] == "distract":
        distract(float(a[1]), opt("--freq", 1000), min(opt("--amp", 0.05), 0.05))
    elif a[0] == "links":
        print(json.dumps(dict(zip(["feeding_links", "mic_input_ports"], feeders()))))
