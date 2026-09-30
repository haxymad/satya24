import struct, zlib

w = h = 512
# Pink background with a white circle
buf = bytearray()
for y in range(h):
    for x in range(w):
        dx, dy = x - 256, y - 256
        if dx*dx + dy*dy < 180*180:
            buf += bytes([255, 255, 255, 255])  # white
        else:
            buf += bytes([233, 69, 96, 255])    # accent

def chunk(typ, data):
    c = typ + data
    return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xffffffff)

sig = b"\x89PNG\r\n\x1a\n"
ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)

raw = b""
for y in range(h):
    raw += b"\x00"
    raw += bytes(buf[y*w*4 : (y+1)*w*4])
idat = zlib.compress(raw, 9)

png = sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")
open("icons/icon.png", "wb").write(png)
print(f"wrote icons/icon.png ({len(png)} bytes)")
