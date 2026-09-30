"""FFmpeg shell-out. The Rust satya-video crate does the same thing;
this exists so pure-Python callers have no compile-time dep."""
from __future__ import annotations

import shutil
import subprocess
from pathlib import Path


class VideoError(RuntimeError):
    pass


def h264_to_mp4(h264_path: str | Path, mp4_path: str | Path) -> Path:
    h264_path = Path(h264_path)
    mp4_path = Path(mp4_path)
    if not h264_path.exists():
        raise VideoError(f"not found: {h264_path}")

    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise VideoError("ffmpeg not found in PATH")

    mp4_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        ffmpeg, "-y", "-fflags", "+genpts", "-r", "25",
        "-i", str(h264_path),
        "-c:v", "copy", "-bsf:v", "h264_mp4toannexb",
        "-movflags", "+faststart",
        str(mp4_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise VideoError(r.stderr)
    return mp4_path
