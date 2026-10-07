#!/usr/bin/env python3
"""Rough end-to-end latency + long-run drift check: phone (this script) -> AppImage -> vibe_phone_mic -> parec.

Acts as a phone on --port, streams silence, then every --period seconds a 100 ms 1 kHz burst.
Reads `parec -d vibe_phone_mic` with small latency and timestamps chunk arrivals, so
latency(burst) = arrival time of the burst onset in the recording - time the burst was sent.
Also reports dropouts (exact-zero 10 ms blocks inside a burst) over the whole run.
Usage: latency.py PORT APPIMAGE SECS [--period 2] [--rate 48000] [--channels 1]
"""
import json, math, socket, struct, subprocess, sys, threading, time

port, app, secs = int(sys.argv[1]), sys.argv[2], float(sys.argv[3])
def opt(n, d):
    return type(d)(sys.argv[sys.argv.index(n) + 1]) if n in sys.argv else d
period, rate, ch = opt("--period", 2.0), opt("--rate", 48000), opt("--channels", 1)
sent = []  # monotonic time of each burst start
mode = opt("--mode", "burst")  # burst | tone (continuous 997 Hz for click analysis)

def phone():
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", port)); s.listen(1)
    c, _ = s.accept(); c.recv(8)
    h = json.dumps({"name": "Latency Sim", "sampleRate": rate, "channels": ch, "format": "s16le",
                    "frameMs": 10, "version": 1}).encode()
    c.sendall(struct.pack(">BI", 1, len(h)) + h)
    st = json.dumps({"muted": False, "volume": 100, "input": "x"}).encode()
    c.sendall(struct.pack(">BI", 3, len(st)) + st)
    spf = rate // 100; t0 = time.monotonic(); seq = 0; ph = 0
    burst_frames = 10; per = int(period * 100)
    try:
        while True:
            k = seq % per
            if mode == "tone":
                vals = [int(0.3 * 32767 * math.sin(2 * math.pi * 997 * (ph + i) / rate)) for i in range(spf)]
            elif 50 <= k < 50 + burst_frames:
                if k == 50:
                    sent.append(time.monotonic())
                vals = [int(0.3 * 32767 * math.sin(2 * math.pi * 1000 * (ph + i) / rate)) for i in range(spf)]
            else:
                vals = [0] * spf
            ph += spf
            pcm = struct.pack("<%dh" % (spf * ch), *[v for v in vals for _ in range(ch)])
            p = struct.pack(">IQ", seq, 0) + pcm
            c.sendall(struct.pack(">BI", 2, len(p)) + p)
            seq += 1
            d = t0 + seq * 0.01 - time.monotonic()
            if d > 0:
                time.sleep(d)
    except OSError:
        pass

threading.Thread(target=phone, daemon=True).start()
time.sleep(0.2)
ap = subprocess.Popen([app, "--headless", "--connect", f"127.0.0.1:{port}", "--duration", str(secs + 1)],
                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
for _ in range(100):
    if "vibe_phone_mic\t" in subprocess.run(["pactl", "list", "short", "sources"], capture_output=True, text=True).stdout:
        break
    time.sleep(0.05)
else:
    sys.exit("vibe_phone_mic source never appeared")
rec = subprocess.Popen(["parec", "--latency-msec=10", "-d", "vibe_phone_mic", "--format=s16le", "--rate=48000",
                        "--channels=1"], stdout=subprocess.PIPE)
samples = []; arrivals = []  # (sample_index_after_chunk, time)
end = time.monotonic() + secs
srcidx = None
while time.monotonic() < end:
    b = rec.stdout.read1(960) if hasattr(rec.stdout, "read1") else rec.stdout.read(960)
    now = time.monotonic()
    samples.extend(struct.unpack("<%dh" % (len(b) // 2), b[: len(b) // 2 * 2]))
    arrivals.append((len(samples), now))
    if srcidx is None and len(samples) > 4800:
        out = subprocess.run(["pactl", "list", "short", "source-outputs"], capture_output=True, text=True).stdout
        srcs = subprocess.run(["pactl", "list", "short", "sources"], capture_output=True, text=True).stdout
        idx = [l.split("\t")[0] for l in srcs.splitlines() if l.split("\t")[1] == "vibe_phone_mic"]
        srcidx = idx[0] if idx and any(l.split("\t")[1] == idx[0] for l in out.splitlines()) else "NOT-ATTACHED"
rec.terminate()
ap.wait()
summary = [l for l in ap.stdout.read().splitlines() if l.startswith("SUMMARY")]
blk = 480
blocks = [samples[i:i + blk] for i in range(0, len(samples) - blk + 1, blk)]
loud = [max(abs(v) for v in b) > 3000 for b in blocks]
# onsets: first loud block after >= 20 quiet blocks; refine to first sample > 3000
onsets = []
for i in range(1, len(blocks)):
    if loud[i] and not any(loud[max(0, i - 20):i]):
        j = next(k for k, v in enumerate(blocks[i]) if abs(v) > 3000)
        onsets.append(i * blk + j)
def time_of(idx):
    # time when sample idx arrived (first chunk whose end >= idx)
    for n, t in arrivals:
        if n > idx:
            return t
    return None
lat = []
for o in onsets:
    t = time_of(o)
    # match with the latest burst sent before t
    cands = [s for s in sent if s <= t]
    if cands:
        lat.append(round((t - cands[-1]) * 1000, 1))
res = {"recorded_s": round(len(samples) / 48000, 1), "attached_source_idx": srcidx, "bursts_sent": len(sent),
       "onsets_seen": len(onsets), "latency_ms": lat, "summary": summary}
if mode == "tone":
    x = samples[48000:]
    w = 2 * math.pi * 997 / 48000; k = 2 - 2 * math.cos(w)
    resid = [abs(x[n] - 2 * x[n - 1] + x[n - 2] + k * x[n - 1]) for n in range(2, len(x))]
    bl = [math.sqrt(sum(v * v for v in x[i:i + blk]) / blk) for i in range(0, len(x) - blk + 1, blk)]
    med = sorted(bl)[len(bl) // 2] if bl else 0
    res.update({"tone_blocks": len(bl), "median_block_rms": round(med, 1),
                "blocks_off_by_10pct": sum(1 for v in bl if abs(v - med) > 0.1 * med),
                "zero_blocks": sum(1 for v in bl if v == 0),
                "max_hp_residual": max(resid) if resid else None,
                "residual_spikes_gt_300": sum(1 for v in resid if v > 300)})
print(json.dumps(res))
