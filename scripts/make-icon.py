#!/usr/bin/env python3
"""Draw Oxicord's icon (a white speech bubble on a blue tile) with no dependencies."""
import struct, sys, zlib

N = 1024

def rrect(x, y, cx, cy, hw, hh, r):
    """Signed distance to a rounded rectangle."""
    dx, dy = abs(x - cx) - (hw - r), abs(y - cy) - (hh - r)
    outside = (max(dx, 0) ** 2 + max(dy, 0) ** 2) ** 0.5
    return outside + min(max(dx, dy), 0) - r

def cover(d):
    return min(1.0, max(0.0, 0.5 - d))

def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))

rows = bytearray()
for y in range(N):
    rows.append(0)
    for x in range(N):
        tile = cover(rrect(x, y, 512, 512, 412, 412, 185))
        col = mix((64, 156, 255), (22, 82, 214), y / N)
        bubble = rrect(x, y, 512, 470, 250, 190, 110)
        # tail: a small rounded square tucked under the bubble's left side
        tail = rrect(x, y, 372, 650, 62, 62, 18)
        white = cover(min(bubble, tail))
        col = mix(col, (255, 255, 255), white)
        for ex in (422, 602):  # eyes
            eye = cover(rrect(x, y, ex, 470, 42, 58, 42))
            col = mix(col, (28, 96, 226), eye)
        rows += bytes((round(col[0]), round(col[1]), round(col[2]), round(255 * tile)))

def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b"")
open(sys.argv[1] if len(sys.argv) > 1 else "assets/icon.png", "wb").write(png)
