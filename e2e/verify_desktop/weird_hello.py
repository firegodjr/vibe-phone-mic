#!/usr/bin/env python3
"""Serve one stream connection with a custom HELLO JSON then 1 s of frames. argv: port hello_json samples_per_frame"""
import socket, struct, sys, time, json
port, hello, spf = int(sys.argv[1]), sys.argv[2].encode(), int(sys.argv[3])
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", port)); s.listen(1)
c, _ = s.accept(); c.recv(8)
c.sendall(struct.pack(">BI", 1, len(hello)) + hello)
try:
    for i in range(100):
        p = struct.pack(">IQ", i, 0) + struct.pack("<%dh" % spf, *([1000] * spf))
        c.sendall(struct.pack(">BI", 2, len(p)) + p); time.sleep(0.01)
except OSError: pass
c.close()
