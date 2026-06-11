#!/usr/bin/env python3
"""Drive one booted Eternal Typewriter session over the QEMU monitor and
capture framebuffer snapshots, for assembling the README demo GIF.

Connects to QEMU's human monitor on a unix socket, holds on the freshly
booted page for a few frames, types `text` one character at a time (snapping
a screendump every other character for a typewriter feel), then settles on
the finished line. Prints the next free frame index so a second invocation
can continue the numbering across a reboot.

Used by scripts/record_demo.sh; not meant to be run by hand.

Usage: record_demo.py SOCKET FRAMES_DIR START_IDX TEXT INTRO_HOLDS FINAL_HOLDS
"""
import os
import socket
import sys
import time

sock_path = sys.argv[1]
frames_dir = sys.argv[2]
start_idx = int(sys.argv[3])
text = sys.argv[4]
intro_holds = int(sys.argv[5])
final_holds = int(sys.argv[6])

s = socket.socket(socket.AF_UNIX)
s.connect(sock_path)
time.sleep(0.3)
try:
    s.recv(65536)  # monitor banner
except Exception:
    pass

idx = start_idx


def dump():
    global idx
    path = os.path.join(frames_dir, "f%04d.ppm" % idx)
    s.sendall(("screendump %s\n" % path).encode())
    time.sleep(0.25)
    try:
        s.recv(65536)
    except Exception:
        pass
    idx += 1


def key(k):
    s.sendall(("sendkey %s\n" % k).encode())
    time.sleep(0.05)
    try:
        s.recv(65536)
    except Exception:
        pass


# Hold on the page as it booted.
for _ in range(intro_holds):
    dump()
    time.sleep(0.12)

# Type the line, capturing a frame every two characters.
keymap = {" ": "spc"}
for i, ch in enumerate(text):
    key(keymap.get(ch, ch))
    if i % 2 == 1:
        time.sleep(0.07)
        dump()

# Settle on the finished line.
for _ in range(final_holds):
    time.sleep(0.12)
    dump()

print(idx)
s.sendall(b"quit\n")
time.sleep(0.2)
