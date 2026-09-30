#!/usr/bin/env python3
"""Generate synthetic DVR images for testing."""
import struct, os, sys

def make_hikvision(path: str, n: int = 200):
    with open(path, 'wb') as f:
        master = bytearray(2048)
        master[0:18] = b"HIKVISION@HANGZHOU"
        struct.pack_into('<Q', master, 0x260, 2048)       # log offset
        struct.pack_into('<Q', master, 0x268, n * 32)     # log size
        f.write(master)

        for i in range(n):
            rec = bytearray(32)
            rec[0:4] = b"RATS"
            struct.pack_into('<I', rec, 4, 0x14000000)
            struct.pack_into('<I', rec, 8, 1700000000 + i * 10)
            struct.pack_into('<H', rec, 12, 0x0002)
            struct.pack_into('<H', rec, 14, 0x0001)
            f.write(rec)

        for i in range(n):
            f.write(b'\x00\x00\x00\x01\x65' + os.urandom(1024))
    print(f"Wrote {path} ({os.path.getsize(path)} bytes)")

def make_dahua(path: str, n: int = 200):
    with open(path, 'wb') as f:
        sb = bytearray(512)
        sb[0:8] = b"DHFS4.1"
        struct.pack_into('<I', sb, 8, 0x00040001)
        struct.pack_into('<I', sb, 12, 4096)
        struct.pack_into('<Q', sb, 16, n)
        f.write(sb)
        f.write(b'\x00' * (33 * 512))

        for i in range(n):
            hdr = bytearray(20)
            struct.pack_into('<I', hdr, 0, 0x44484653)
            struct.pack_into('<H', hdr, 4, 0x0001)
            struct.pack_into('<I', hdr, 6, 1700000000 + i * 10)
            struct.pack_into('<I', hdr, 10, 1024)
            struct.pack_into('<I', hdr, 14, 0x44484653)
            f.write(hdr)
            f.write(b'\x00\x00\x00\x01\x65' + os.urandom(1019))
    print(f"Wrote {path} ({os.path.getsize(path)} bytes)")

if __name__ == "__main__":
    os.makedirs("data/test_images", exist_ok=True)
    make_hikvision("data/test_images/synthetic_hikvision.img")
    make_dahua("data/test_images/synthetic_dahua.img")
