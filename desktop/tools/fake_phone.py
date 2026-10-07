#!/usr/bin/env python3
"""Fake Vibe Phone Mic server (protocol v1), python3 stdlib only.

Answers UDP discovery and streams a sine wave over TCP, exactly like the phone app.

  python3 fake_phone.py [--port 48777] [--freq 440] [--rate 48000] [--channels 1]
                        [--name "Fake Phone"] [--amplitude 0.3] [--volume 100]
                        [--mute-toggle SECS] [--muted] [--no-udp] [--bind 0.0.0.0]
"""
import argparse
import json
import math
import socket
import struct
import sys
import threading
import time

DISCOVER = b"VPMIC_DISCOVER_V1"
HELLO, AUDIO, STATUS, BYE = 0x01, 0x02, 0x03, 0x04


def log(*a):
    print("[fake_phone]", *a, file=sys.stderr, flush=True)


def frame(kind, payload):
    return struct.pack(">BI", kind, len(payload)) + payload


class Phone:
    def __init__(self, args):
        self.a = args
        self.lock = threading.Lock()
        self.muted = args.muted
        self.volume = args.volume
        self.current = None  # current streaming socket
        self.generation = 0

    def hello(self):
        return frame(HELLO, json.dumps({
            "name": self.a.name, "sampleRate": self.a.rate, "channels": self.a.channels,
            "format": "s16le", "frameMs": 10, "version": 1}).encode())

    def status(self):
        return frame(STATUS, json.dumps({
            "muted": self.muted, "volume": self.volume, "input": "Fake sine generator"}).encode())

    def udp_loop(self):
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind((self.a.bind, self.a.port))
        log(f"UDP discovery on {self.a.bind}:{self.a.port}")
        while True:
            data, addr = s.recvfrom(2048)
            if data == DISCOVER:
                reply = json.dumps({"service": "vibe-phone-mic", "version": 1,
                                    "name": self.a.name, "port": self.a.port}).encode()
                s.sendto(reply, addr)
                log(f"discovery reply to {addr[0]}:{addr[1]}")

    def tcp_loop(self):
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind((self.a.bind, self.a.port))
        srv.listen(8)
        log(f"TCP stream on {self.a.bind}:{self.a.port}  freq={self.a.freq} Hz rate={self.a.rate}")
        while True:
            c, addr = srv.accept()
            threading.Thread(target=self.client, args=(c, addr), daemon=True).start()

    def client(self, c, addr):
        try:
            c.settimeout(3)
            hs = b""
            while len(hs) < 8:
                chunk = c.recv(8 - len(hs))
                if not chunk:
                    raise ConnectionError("closed during handshake")
                hs += chunk
            if hs[:4] != b"VPMC" or hs[4] != 1 or hs[5] not in (0, 1):
                log(f"bad handshake from {addr[0]}: {hs!r}")
                c.close()
                return
            c.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            if hs[5] == 1:
                c.sendall(self.hello())
                c.close()
                log(f"probe from {addr[0]}")
                return
            with self.lock:
                # The previous client (if any) notices the new generation and
                # sends BYE {"reason":"replaced"} itself before closing.
                self.current = c
                self.generation += 1
                gen = self.generation
            log(f"stream client {addr[0]}:{addr[1]}")
            c.settimeout(None)
            c.sendall(self.hello() + self.status())
            self.stream(c, gen)
        except (OSError, ConnectionError) as e:
            log(f"client {addr[0]} ended: {e}")
        finally:
            try:
                c.close()
            except OSError:
                pass

    def stream(self, c, gen):
        a = self.a
        n = a.rate // 100  # 10 ms
        phase = 0.0
        step = 2 * math.pi * a.freq / a.rate
        seq = 0
        t0 = time.monotonic()
        last_toggle = t0
        while True:
            with self.lock:
                if gen != self.generation:
                    c.sendall(frame(BYE, b'{"reason":"replaced"}'))
                    return
            now = time.monotonic()
            if a.mute_toggle and now - last_toggle >= a.mute_toggle:
                last_toggle = now
                self.muted = not self.muted
                log(f"muted={self.muted}")
                c.sendall(self.status())
            gain = 0.0 if self.muted else a.amplitude * self.volume / 100.0
            samples = []
            for _ in range(n):
                v = int(max(-1.0, min(1.0, math.sin(phase) * gain)) * 32767)
                samples.extend([v] * a.channels)
                phase += step
            phase %= 2 * math.pi
            pcm = struct.pack("<%dh" % len(samples), *samples)
            payload = struct.pack(">IQ", seq & 0xFFFFFFFF, int(time.time() * 1e6)) + pcm
            c.sendall(frame(AUDIO, payload))
            seq += 1
            # Pace in real time.
            target = t0 + seq * 0.01
            delay = target - time.monotonic()
            if delay > 0:
                time.sleep(delay)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--port", type=int, default=48777)
    p.add_argument("--bind", default="0.0.0.0")
    p.add_argument("--freq", type=float, default=440.0)
    p.add_argument("--rate", type=int, default=48000)
    p.add_argument("--channels", type=int, default=1)
    p.add_argument("--name", default="Fake Phone")
    p.add_argument("--amplitude", type=float, default=0.3)
    p.add_argument("--volume", type=int, default=100)
    p.add_argument("--muted", action="store_true")
    p.add_argument("--mute-toggle", type=float, default=0.0, help="toggle mute every N seconds")
    p.add_argument("--no-udp", action="store_true")
    args = p.parse_args()
    ph = Phone(args)
    if not args.no_udp:
        threading.Thread(target=ph.udp_loop, daemon=True).start()
    try:
        ph.tcp_loop()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
