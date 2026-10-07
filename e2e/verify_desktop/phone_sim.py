#!/usr/bin/env python3
"""Independent fake phone server written from PLAN.md wire protocol v1 (stdlib only).

Written by the desktop verifier, deliberately NOT derived from desktop/tools/fake_phone.py.

Behaviours (--behavior):
  normal       stream sine forever (until client goes away)
  mute:N       after N audio frames send STATUS muted=true and zero PCM thereafter
  close:N      close the socket after N audio frames
  garbage      send random non-protocol bytes instead of HELLO, then close
  oversize     send HELLO, STATUS, then an AUDIO header with payload len 1 MiB + 1
  exact1mib    send HELLO, STATUS, then one AUDIO frame with payload exactly 1 MiB, then normal
  stall:N      after N frames stop sending but keep the socket open
  nohello      send STATUS first (no HELLO)
  volume:N     after N frames STATUS volume=50 and halve amplitude
  bye-stopped:N  after N frames send BYE {"reason":"stopped"} and close
On replacement of a stream client the old one gets BYE {"reason":"replaced"} (unless --no-bye).
"""
import argparse
import json
import math
import os
import socket
import struct
import sys
import threading
import time

MAGIC = b"VPMC"


def log(*a):
    print("[phone_sim]", *a, file=sys.stderr, flush=True)


def frame(kind, payload):
    return struct.pack(">BI", kind, len(payload)) + payload


class Sim:
    def __init__(self, a):
        self.a = a
        self.lock = threading.Lock()
        self.current = None  # current streaming socket
        self.send_locks = {}  # socket -> lock (frames from 2 threads must not interleave)
        self.stats = {"probes": 0, "streams": 0, "bad_handshakes": 0, "frames_sent": 0}

    def hello(self):
        return frame(1, json.dumps({
            "name": self.a.name, "sampleRate": self.a.rate, "channels": self.a.channels,
            "format": "s16le", "frameMs": 10, "version": 1}).encode())

    def status(self, muted=False, volume=100):
        return frame(3, json.dumps({"muted": muted, "volume": volume, "input": "Sim microphone"}).encode())

    def bye(self, reason):
        return frame(4, json.dumps({"reason": reason}).encode())

    def send(self, conn, data):
        with self.send_locks.setdefault(conn, threading.Lock()):
            conn.sendall(data)

    def handle(self, conn, peer):
        conn.settimeout(5)
        try:
            hs = b""
            while len(hs) < 8:
                chunk = conn.recv(8 - len(hs))
                if not chunk:
                    break
                hs += chunk
        except OSError:
            hs = b""
        log("handshake from", peer, hs.hex())
        if len(hs) != 8 or hs[:4] != MAGIC or hs[4] != 1 or hs[5] not in (0, 1) or hs[6:8] != b"\0\0":
            self.stats["bad_handshakes"] += 1
            log("BAD handshake, closing")
            conn.close()
            return
        mode = hs[5]
        if mode == 1:
            self.stats["probes"] += 1
            conn.sendall(self.hello())
            conn.close()
            return
        self.stats["streams"] += 1
        with self.lock:
            old, self.current = self.current, conn
        if old is not None:
            log("replacing previous stream client")
            try:
                if not self.a.no_bye:
                    self.send(old, self.bye("replaced"))
                    log("sent BYE replaced")
            except OSError:
                pass
            try:
                old.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            old.close()
        self.stream(conn)

    def stream(self, conn):
        a = self.a
        beh, _, arg = a.behavior.partition(":")
        n_arg = int(arg) if arg else 0
        try:
            if beh == "garbage":
                self.send(conn, b"HTTP/1.1 200 OK\r\n\r\n" + os.urandom(64))
                time.sleep(0.2)
                conn.close()
                return
            if beh == "nohello":
                self.send(conn, self.status())
                time.sleep(1)
                conn.close()
                return
            self.send(conn, self.hello())
            self.send(conn, self.status())
            if beh == "oversize":
                self.send(conn, struct.pack(">BI", 2, (1 << 20) + 1) + b"\0" * 4096)
                time.sleep(2)
                conn.close()
                return
            if beh == "exact1mib":
                payload = struct.pack(">IQ", 0, 0) + b"\0" * ((1 << 20) - 12)
                self.send(conn, frame(2, payload))
            spf = a.rate // 100  # samples per channel per 10 ms frame
            seq = 1 if beh == "exact1mib" else 0
            phase = 0
            muted = False
            amp = a.amp
            t0 = time.monotonic()
            while True:
                if beh == "close" and seq >= n_arg:
                    log("closing after", seq, "frames")
                    conn.close()
                    return
                if beh == "stall" and seq >= n_arg:
                    log("stalling")
                    time.sleep(30)
                    conn.close()
                    return
                if beh == "mute" and seq == n_arg:
                    muted = True
                    self.send(conn, self.status(muted=True))
                if beh == "bye-stopped" and seq >= n_arg:
                    self.send(conn, self.bye("stopped"))
                    log("sent BYE stopped after", seq, "frames")
                    conn.close()
                    return
                if beh == "volume" and seq == n_arg:
                    amp = amp / 2
                    self.send(conn, self.status(volume=50))
                samples = []
                for i in range(spf):
                    v = 0 if muted else int(amp * 32767 * math.sin(2 * math.pi * a.freq * (phase + i) / a.rate))
                    samples.extend([v] * a.channels)
                phase += spf
                pcm = struct.pack("<%dh" % len(samples), *samples)
                self.send(conn, frame(2, struct.pack(">IQ", seq, int(time.time() * 1e6)) + pcm))
                seq += 1
                self.stats["frames_sent"] = seq
                # real-time pacing
                target = t0 + seq * 0.01
                d = target - time.monotonic()
                if d > 0:
                    time.sleep(d)
        except OSError as e:
            log("stream ended:", e)
        finally:
            with self.lock:
                if self.current is conn:
                    self.current = None
            log("stats", json.dumps(self.stats))

    def udp_loop(self):
        u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        u.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        u.bind((self.a.udp_host, self.a.udp_port))
        log("udp discovery on", (self.a.udp_host, self.a.udp_port))
        while True:
            data, src = u.recvfrom(2048)
            log("udp from", src, data)
            if data == b"VPMIC_DISCOVER_V1":
                u.sendto(json.dumps({"service": "vibe-phone-mic", "version": 1, "name": self.a.name,
                                     "port": self.a.port}).encode(), src)

    def run(self):
        if self.a.udp_port:
            threading.Thread(target=self.udp_loop, daemon=True).start()
        s = socket.socket()
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind((self.a.host, self.a.port))
        s.listen(8)
        log("tcp listening on", (self.a.host, self.a.port), "behavior", self.a.behavior)
        print("READY", flush=True)
        while True:
            c, p = s.accept()
            threading.Thread(target=self.handle, args=(c, p), daemon=True).start()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=48790)
    ap.add_argument("--udp-host", default="0.0.0.0")
    ap.add_argument("--udp-port", type=int, default=0)
    ap.add_argument("--name", default="Verifier Sim")
    ap.add_argument("--rate", type=int, default=48000)
    ap.add_argument("--channels", type=int, default=1)
    ap.add_argument("--freq", type=float, default=523.0)
    ap.add_argument("--amp", type=float, default=0.5)
    ap.add_argument("--behavior", default="normal")
    ap.add_argument("--no-bye", action="store_true", help="old-phone mode: never send BYE")
    a = ap.parse_args()
    if a.port == 48777 and a.host in ("127.0.0.1", "0.0.0.0"):
        sys.exit("refusing to bind 127.0.0.1/0.0.0.0:48777 (adb forward in use)")
    Sim(a).run()


if __name__ == "__main__":
    main()
