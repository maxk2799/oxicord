#!/usr/bin/env python3
"""Recolour a blurple-and-white app icon to a dark tile with a blurple mark.

Usage: dark-icon.py in.png out.png   (8-bit RGB/RGBA PNG, no dependencies)
"""
import struct, sys, zlib

def read_png(path):
    data = open(path, "rb").read()
    pos, idat, w = 8, b"", None
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and ctype in (2, 6) and interlace == 0, "unsupported PNG"
        elif kind == b"IDAT":
            idat += body
        pos += n + 12
    bpp = 4 if ctype == 6 else 3
    raw, stride = zlib.decompress(idat), w * bpp
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        cur = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = cur[i - bpp] if i >= bpp else 0
            b, c = prev[i], (prev[i - bpp] if i >= bpp else 0)
            if f == 1: cur[i] = (cur[i] + a) & 255
            elif f == 2: cur[i] = (cur[i] + b) & 255
            elif f == 3: cur[i] = (cur[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                cur[i] = (cur[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(cur)
        prev = cur
    return w, h, bpp, rows

def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

w, h, bpp, rows = read_png(sys.argv[1])
DARK_TOP, DARK_BOTTOM, MARK = (43, 45, 49), (24, 25, 28), (88, 101, 242)
out = bytearray()
for y, row in enumerate(rows):
    out.append(0)
    tile = [DARK_TOP[i] + (DARK_BOTTOM[i] - DARK_TOP[i]) * y / h for i in range(3)]
    for x in range(w):
        px = row[x * bpp:(x + 1) * bpp]
        r, g, b = px[0], px[1], px[2]
        alpha = px[3] if bpp == 4 else 255
        # How white the pixel is: the tile is saturated, the mark is not.
        white = max(0.0, min(1.0, (min(r, g, b) - 110) / 120))
        out += bytes(round(tile[i] + (MARK[i] - tile[i]) * white) for i in range(3)) + bytes((alpha,))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(bytes(out), 9)) + chunk(b"IEND", b"")
open(sys.argv[2], "wb").write(png)
