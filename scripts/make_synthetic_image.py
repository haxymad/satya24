#!/usr/bin/env python3
"""
Generate synthetic DVR images with the H.264 payload stored as ONE
contiguous block. This avoids the carving ambiguity that breaks
FFmpeg when NAL units are split across metadata records.
"""
import struct, os, sys
from datetime import datetime, timezone

def load_h264(path):
    data = open(path, "rb").read()
    print(f"  Loaded {len(data):,} bytes of H.264")
    return data

def make_hikvision(path, h264):
    # Master sector (2048) + log area (fixed 6400) + contiguous H.264
    with open(path, 'wb') as f:
        master = bytearray(2048)
        master[0:18] = b"HIKVISION@HANGZHOU"
        LOG_OFF = 2048
        LOG_SIZE = 6400
        struct.pack_into('<Q', master, 0x260, LOG_OFF)
        struct.pack_into('<Q', master, 0x268, LOG_SIZE)
        f.write(master)

        # 200 log records (200 * 32 = 6400 bytes)
        base = 1700000000
        for i in range(200):
            rec = bytearray(32)
            rec[0:8] = bytes([0x52, 0x41, 0x54, 0x53, 0x01, 0x00, 0x00, 0x00])
            struct.pack_into('<I', rec, 8, base + i * 10)
            struct.pack_into('<H', rec, 12, 0x0002)
            struct.pack_into('<H', rec, 14, 0x0001)
            f.write(rec)

        # Contiguous H.264 — ONE block, no metadata breaks
        f.write(h264)

    print(f"  ✓ Hikvision     {path}  ({os.path.getsize(path):>12,} bytes)")

def make_dahua(path, h264):
    SECTOR = 512
    PART = 128 * SECTOR
    with open(path, 'wb') as f:
        s0 = bytearray(SECTOR); s0[0:7] = b"DHFS4.1"; f.write(s0)
        f.write(b'\x00' * (0x3C00 - SECTOR))
        f.write(b'\x00' * 0x34)
        e = bytearray(64)
        struct.pack_into('<I', e, 20, 0)
        struct.pack_into('<Q', e, 48, PART // 512)
        f.write(e)
        t = bytearray(64); t[0:4] = bytes([0xAA, 0x55, 0xAA, 0x55]); f.write(t)
        cur = 0x3C00 + 0x34 + 128
        f.write(b'\x00' * (PART - cur))
        sb = bytearray(0x100)
        struct.pack_into('<I', sb, 0x2c, 4096)
        struct.pack_into('<I', sb, 0x30, 512)
        f.write(sb)
        # Contiguous H.264
        f.write(h264)
    print(f"  ✓ Dahua         {path}  ({os.path.getsize(path):>12,} bytes)")

def make_wfs(path, h264, version=b"WFS0.4"):
    with open(path, 'wb') as f:
        hdr = bytearray(1024)
        hdr[0:6] = version
        struct.pack_into('<I', hdr, 8, 200)
        struct.pack_into('<I', hdr, 12, 4096)
        f.write(hdr)
        f.write(h264)
    label = "WFS0.4" if b"0.4" in version else "WFS0.5"
    print(f"  ✓ {label:<11} {path}  ({os.path.getsize(path):>12,} bytes)")

def make_cpplus(path, h264):
    with open(path, 'wb') as f:
        boot = bytearray(512)
        boot[0:3] = b'\xEB\x58\x90'
        boot[3:11] = b"MSWIN4.1"
        boot[82:87] = b"FAT32"
        boot[510:512] = b'\x55\xAA'
        f.write(boot)
        f.write(b'\x00' * (1024 - 512))
        f.write(h264)
    print(f"  ✓ CP Plus       {path}  ({os.path.getsize(path):>12,} bytes)")

def make_honeywell(path, h264):
    with open(path, 'wb') as f:
        hdr = bytearray(512)
        hdr[0:9] = b"HONEYWELL"
        struct.pack_into('<I', hdr, 16, 200)
        f.write(hdr)
        f.write(b'\x00' * 512)
        f.write(h264)
    print(f"  ✓ Honeywell     {path}  ({os.path.getsize(path):>12,} bytes)")

def make_uniview(path, h264):
    with open(path, 'wb') as f:
        hdr = bytearray(512)
        hdr[0:3] = b"UNV"
        struct.pack_into('<I', hdr, 8, 512)
        struct.pack_into('<I', hdr, 12, 200)
        f.write(hdr)
        f.write(h264)
    print(f"  ✓ Uniview       {path}  ({os.path.getsize(path):>12,} bytes)")

def make_tplink(path, h264):
    with open(path, 'wb') as f:
        f.write(b'\x00' * 1024)
        sb = bytearray(1024)
        struct.pack_into('<H', sb, 0x38, 0xEF53)
        f.write(sb)
        hdr = bytearray(256)
        hdr[0:4] = b"TPOS"
        struct.pack_into('<I', hdr, 8, 200)
        f.write(hdr)
        f.write(h264)
    print(f"  ✓ TP-Link       {path}  ({os.path.getsize(path):>12,} bytes)")

def make_godrej(path, h264):
    with open(path, 'wb') as f:
        boot = bytearray(512)
        boot[82:87] = b"FAT32"
        boot[510:512] = b'\x55\xAA'
        f.write(boot)
        idx = bytearray(1024)
        idx[0:6] = b"GODREJ"
        struct.pack_into('<I', idx, 8, 200)
        f.write(idx)
        f.write(h264)
    print(f"  ✓ Godrej        {path}  ({os.path.getsize(path):>12,} bytes)")

def make_matrix(path, h264):
    with open(path, 'wb') as f:
        hdr = bytearray(512)
        hdr[0:7] = b"SATATYA"
        struct.pack_into('<I', hdr, 8, 200)
        struct.pack_into('<H', hdr, 12, 4)
        f.write(hdr)
        f.write(h264)
    print(f"  ✓ Matrix        {path}  ({os.path.getsize(path):>12,} bytes)")

if __name__ == "__main__":
    h264_path = sys.argv[1] if len(sys.argv) > 1 else "data/rick.h264"
    out = sys.argv[2] if len(sys.argv) > 2 else "/home/haxy/satya-images"
    os.makedirs(out, exist_ok=True)

    h264 = load_h264(h264_path)
    print()
    make_hikvision(f"{out}/synthetic_hikvision.img", h264)
    make_dahua    (f"{out}/synthetic_dahua.img", h264)
    make_wfs      (f"{out}/synthetic_wfs04.img", h264, b"WFS0.4")
    make_wfs      (f"{out}/synthetic_wfs05.img", h264, b"WFS0.5")
    make_cpplus   (f"{out}/synthetic_cpplus.img", h264)
    make_honeywell(f"{out}/synthetic_honeywell.img", h264)
    make_uniview  (f"{out}/synthetic_uniview.img", h264)
    make_tplink   (f"{out}/synthetic_tplink.img", h264)
    make_godrej   (f"{out}/synthetic_godrej.img", h264)
    make_matrix   (f"{out}/synthetic_matrix.img", h264)
    print()
    print("  Done — 10 images with contiguous H.264 payload")
