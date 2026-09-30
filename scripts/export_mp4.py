#!/usr/bin/env python3
"""Convert raw H.264 to MP4 using FFmpeg."""
import shutil
import subprocess
import sys
from pathlib import Path


def main():
    if len(sys.argv) != 3:
        sys.exit(f"Usage: {sys.argv[0]} <input.h264> <output.mp4>")

    h264_path = Path(sys.argv[1])
    mp4_path = Path(sys.argv[2])

    if not h264_path.exists():
        sys.exit(f"Not found: {h264_path}")

    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        sys.exit("ffmpeg not found in PATH")

    mp4_path.parent.mkdir(parents=True, exist_ok=True)

    cmd = [
        ffmpeg,
        "-y",
        "-fflags", "+genpts",
        "-r", "25",
        "-i", str(h264_path),
        "-c:v", "copy",
        "-bsf:v", "h264_mp4toannexb",
        "-movflags", "+faststart",
        str(mp4_path),
    ]

    print(f"Running: {' '.join(cmd)}")
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        sys.exit(1)

    print(f"Wrote {mp4_path} ({mp4_path.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
