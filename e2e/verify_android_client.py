#!/usr/bin/env python3
"""Independent stdlib-only client for Vibe Phone Mic wire protocol v1 (see PLAN.md).

Subcommands:
  probe    HOST[:PORT]                      -> handshake mode=1, print HELLO JSON, verify server closes
  stream   HOST[:PORT] --duration S --wav F -> handshake mode=0, record AUDIO to WAV, print a JSON report
                                             (BYE 0x04 {"reason":...} is reported as "bye" with arrival time)
  discover [--target HOST] [--port P]       -> send VPMIC_DISCOVER_V1 over UDP, print replies
  raw      HOST[:PORT] --hex BYTES          -> send arbitrary handshake bytes, report what comes back

The `stream` report (JSON on stdout) contains: hello, statuses (with arrival time), frame count,
seq gaps, samples-per-frame stats, per-second RMS, zero-frame counts and the close reason.
Exit status: 0 on success, 1 on protocol error / validation failure.
"""
import argparse
import json
import math
import socket
import struct
import sys
import time
import wave

MAGIC = b"VPMC"
VERSION = 1
PORT = 48777
MAX_PAYLOAD = 1 << 20
T_HELLO, T_AUDIO, T_STATUS, T_BYE = 1, 2, 3, 4


def parse_hostport(s):
    if ":" in s:
        h, p = s.rsplit(":", 1)
        return h, int(p)
    return s, PORT


def handshake(mode, version=VERSION):
    return MAGIC + bytes([version, mode, 0, 0])


class Closed(Exception):
    pass


def recv_exact(sock, n):
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise Closed("closed after %d/%d bytes" % (len(buf), n))
        buf += chunk
    return bytes(buf)


def read_frame(sock):
    hdr = recv_exact(sock, 5)
    ftype, length = hdr[0], struct.unpack(">I", hdr[1:])[0]
    if length > MAX_PAYLOAD:
        raise ValueError("payload length %d exceeds 1 MiB" % length)
    return ftype, recv_exact(sock, length)


def parse_audio(payload):
    if len(payload) < 12 or (len(payload) - 12) % 2:
        raise ValueError("bad AUDIO payload length %d" % len(payload))
    seq, ts = struct.unpack(">IQ", payload[:12])
    return seq, ts, payload[12:]


def pcm_rms(pcm):
    n = len(pcm) // 2
    if n == 0:
        return 0.0
    s = struct.unpack("<%dh" % n, pcm)
    return math.sqrt(sum(v * v for v in s) / n) / 32768.0


def dbfs(r):
    return 20 * math.log10(r) if r > 0 else -999.0


def connect(host, port, timeout):
    sock = socket.create_connection((host, port), timeout=timeout)
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    return sock


def cmd_probe(a):
    host, port = parse_hostport(a.host)
    try:
        sock = connect(host, port, a.timeout)
        sock.sendall(handshake(1))
        ftype, payload = read_frame(sock)
    except (OSError, Closed) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1
    out = {"firstType": ftype}
    ok = ftype == T_HELLO
    if ok:
        out["hello"] = json.loads(payload.decode("utf-8"))
    # Server must close right after HELLO.
    try:
        sock.settimeout(a.timeout)
        extra = sock.recv(1)
        out["closedAfterHello"] = extra == b""
        if extra:
            out["extraByte"] = extra.hex()
    except socket.timeout:
        out["closedAfterHello"] = False
    except ConnectionResetError:
        out["closedAfterHello"] = True
    ok = ok and out["closedAfterHello"]
    sock.close()
    out["ok"] = ok
    print(json.dumps(out))
    return 0 if ok else 1


def cmd_raw(a):
    host, port = parse_hostport(a.host)
    sock = connect(host, port, a.timeout)
    sock.sendall(bytes.fromhex(a.hex))
    sock.settimeout(a.timeout)
    got = b""
    closed = False
    try:
        while len(got) < 4096:
            c = sock.recv(4096)
            if not c:
                closed = True
                break
            got += c
    except socket.timeout:
        pass
    except ConnectionResetError:
        closed = True
    print(json.dumps({"bytesReceived": len(got), "closed": closed, "head": got[:16].hex()}))
    return 0


def cmd_stream(a):
    host, port = parse_hostport(a.host)
    sock = connect(host, port, a.timeout)
    sock.sendall(handshake(0))
    t0 = time.monotonic()
    rep = {"hello": None, "statuses": [], "frames": 0, "seqFirst": None, "seqLast": None, "seqGaps": 0,
           "seqGapList": [], "samplesPerFrame": {}, "zeroFrames": 0, "firstFrameType": None,
           "audioBeforeHello": False, "closeReason": None, "bye": None, "tsMonotonic": True}
    pcm_all = bytearray()
    per_sec = {}
    last_seq = None
    last_ts = None
    rate, ch = 48000, 1
    try:
        while time.monotonic() - t0 < a.duration:
            ftype, payload = read_frame(sock)
            now = round(time.monotonic() - t0, 3)
            if rep["firstFrameType"] is None:
                rep["firstFrameType"] = ftype
            if ftype == T_HELLO:
                rep["hello"] = json.loads(payload.decode("utf-8"))
                rate = rep["hello"].get("sampleRate", 48000)
                ch = rep["hello"].get("channels", 1)
            elif ftype == T_STATUS:
                st = json.loads(payload.decode("utf-8"))
                st["_t"] = now
                rep["statuses"].append(st)
                if a.verbose:
                    print("STATUS @%.2fs %s" % (now, json.dumps(st)), file=sys.stderr)
            elif ftype == T_AUDIO:
                if rep["hello"] is None:
                    rep["audioBeforeHello"] = True
                seq, ts, pcm = parse_audio(payload)
                if last_seq is not None and seq != (last_seq + 1) & 0xFFFFFFFF:
                    rep["seqGaps"] += 1
                    if len(rep["seqGapList"]) < 20:
                        rep["seqGapList"].append([last_seq, seq])
                if last_ts is not None and ts < last_ts:
                    rep["tsMonotonic"] = False
                last_seq, last_ts = seq, ts
                if rep["seqFirst"] is None:
                    rep["seqFirst"] = seq
                rep["seqLast"] = seq
                rep["frames"] += 1
                spf = len(pcm) // 2 // max(ch, 1)
                rep["samplesPerFrame"][str(spf)] = rep["samplesPerFrame"].get(str(spf), 0) + 1
                if not any(pcm):
                    rep["zeroFrames"] += 1
                sec = int(now)
                d = per_sec.setdefault(sec, [0.0, 0, 0, 0])  # sumsq, nsamples, frames, zeroframes
                n = len(pcm) // 2
                if n:
                    s = struct.unpack("<%dh" % n, pcm)
                    d[0] += sum(v * v for v in s)
                    d[1] += n
                d[2] += 1
                d[3] += 0 if any(pcm) else 1
                pcm_all += pcm
            elif ftype == T_BYE:
                rep["bye"] = json.loads(payload.decode("utf-8"))
                rep["bye"]["_t"] = now
                if a.verbose:
                    print("BYE @%.2fs %s" % (now, json.dumps(rep["bye"])), file=sys.stderr)
            else:
                rep.setdefault("unknownTypes", []).append(ftype)  # ignored (forward compat)
        rep["closeReason"] = "duration"
    except Closed as e:
        rep["closeReason"] = "server closed: %s" % e
    except (ConnectionResetError, BrokenPipeError) as e:
        rep["closeReason"] = "reset: %s" % e
    except socket.timeout:
        rep["closeReason"] = "timeout"
    finally:
        sock.close()
    rep["elapsed"] = round(time.monotonic() - t0, 3)
    rep["perSecond"] = [
        {"sec": k, "frames": v[2], "zeroFrames": v[3],
         "rms": round(math.sqrt(v[0] / v[1]) / 32768.0, 5) if v[1] else 0.0,
         "dbfs": round(dbfs(math.sqrt(v[0] / v[1]) / 32768.0), 1) if v[1] else -999}
        for k, v in sorted(per_sec.items())]
    total = pcm_rms(bytes(pcm_all))
    rep["rms"] = round(total, 5)
    rep["dbfs"] = round(dbfs(total), 1)
    rep["seconds"] = round(len(pcm_all) / 2 / max(ch, 1) / rate, 3)
    if a.wav:
        with wave.open(a.wav, "wb") as w:
            w.setnchannels(ch)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(bytes(pcm_all))
        rep["wav"] = a.wav
    print(json.dumps(rep, indent=None if a.compact else 1))
    ok = rep["hello"] is not None and rep["firstFrameType"] == T_HELLO and rep["frames"] > 0
    if a.expect_hello_rate:
        ok = ok and rep["hello"].get("sampleRate") == a.expect_hello_rate
    return 0 if ok else 1


def cmd_discover(a):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    s.bind(("0.0.0.0", a.local_port))
    s.settimeout(0.2)
    msg = a.message.encode("ascii")
    s.sendto(msg, (a.target, a.port))
    deadline = time.monotonic() + a.timeout
    replies = []
    while time.monotonic() < deadline:
        try:
            data, addr = s.recvfrom(4096)
        except socket.timeout:
            continue
        try:
            js = json.loads(data.decode("utf-8"))
        except Exception:
            js = {"_raw": data.decode("utf-8", "replace")}
        replies.append({"from": "%s:%d" % addr, "reply": js})
    s.close()
    print(json.dumps(replies))
    ok = any(r["reply"].get("service") == "vibe-phone-mic" for r in replies)
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("probe"); p.add_argument("host"); p.add_argument("--timeout", type=float, default=3.0)
    p.set_defaults(fn=cmd_probe)
    p = sub.add_parser("raw"); p.add_argument("host"); p.add_argument("--hex", required=True)
    p.add_argument("--timeout", type=float, default=4.0); p.set_defaults(fn=cmd_raw)
    p = sub.add_parser("stream"); p.add_argument("host"); p.add_argument("--duration", type=float, default=5.0)
    p.add_argument("--wav"); p.add_argument("--timeout", type=float, default=5.0)
    p.add_argument("--expect-hello-rate", type=int); p.add_argument("--compact", action="store_true")
    p.add_argument("-v", "--verbose", action="store_true"); p.set_defaults(fn=cmd_stream)
    p = sub.add_parser("discover"); p.add_argument("--target", default="255.255.255.255")
    p.add_argument("--port", type=int, default=PORT); p.add_argument("--local-port", type=int, default=0)
    p.add_argument("--timeout", type=float, default=2.0); p.add_argument("--message", default="VPMIC_DISCOVER_V1")
    p.set_defaults(fn=cmd_discover)
    a = ap.parse_args()
    sys.exit(a.fn(a))


if __name__ == "__main__":
    main()
