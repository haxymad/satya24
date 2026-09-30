"""Subprocess wrapper around the Rust `satya` binary."""
from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional


class SatyaError(RuntimeError):
    pass


def _find_binary(name: str = "satya") -> Path:
    """Locate the `satya` CLI. Prefers a debug/release build in the
    project tree, then falls back to PATH."""
    here = Path(__file__).resolve()
    for parent in [here.parent, *here.parents]:
        candidate = parent / "target" / "release" / name
        if candidate.exists():
            return candidate
        candidate = parent / "target" / "debug" / name
        if candidate.exists():
            return candidate
    found = shutil.which(name)
    if found:
        return Path(found)
    raise SatyaError(
        f"`{name}` binary not found. Build it with:\n"
        f"  cargo build --release"
    )


@dataclass
class Frame:
    offset: int
    length: int
    codec: str
    recovery_source: str
    recovery_confidence: float
    claims: list[dict[str, Any]]


class Satya:
    """High-level interface to the Rust core."""

    def __init__(self, binary: Optional[str] = None):
        self.binary = Path(binary) if binary else _find_binary()

    # -- low-level ----------------------------------------------------------

    def _run(self, *args: str) -> str:
        try:
            result = subprocess.run(
                [str(self.binary), *args],
                capture_output=True,
                text=True,
                check=False,
            )
        except FileNotFoundError as e:
            raise SatyaError(str(e)) from e

        if result.returncode != 0:
            raise SatyaError(
                f"satya exited {result.returncode}:\n{result.stderr.strip()}"
            )
        return result.stdout

    # -- commands -----------------------------------------------------------

    def identify(self, image: str | Path) -> dict[str, Any]:
        out = self._run("identify", "-i", str(image))
        info: dict[str, Any] = {}
        for line in out.strip().splitlines():
            if ":" in line:
                k, _, v = line.partition(":")
                info[k.strip().lower().replace(" ", "_")] = v.strip()
        return info

    def enumerate(
        self,
        image: str | Path,
        output_json: Optional[str | Path] = None,
    ) -> list[Frame]:
        args = ["enumerate", "-i", str(image)]
        if output_json:
            args += ["-o", str(output_json)]
            self._run(*args)
            data = json.loads(Path(output_json).read_text())
        else:
            # The CLI prints a summary to stdout; we re-run with -o to a tmp file.
            import tempfile
            with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
                tmp_path = tmp.name
            try:
                self._run(*args, "-o", tmp_path)
                data = json.loads(Path(tmp_path).read_text())
            finally:
                Path(tmp_path).unlink(missing_ok=True)

        return [
            Frame(
                offset=f["offset"],
                length=f["length"],
                codec=f["codec"],
                recovery_source=f["recovery_source"],
                recovery_confidence=f["recovery_confidence"],
                claims=f.get("claims", []),
            )
            for f in data
        ]

    def hash(self, image: str | Path) -> str:
        out = self._run("hash", "-i", str(image))
        for line in out.splitlines():
            if line.lower().startswith("sha-256"):
                return line.split(":", 1)[1].strip()
        raise SatyaError(f"could not parse hash output: {out!r}")

    def export_h264(self, image: str | Path, out_path: str | Path) -> Path:
        self._run("export", "-i", str(image), "-o", str(out_path))
        return Path(out_path)

    def export_mp4(self, image: str | Path, out_mp4: str | Path) -> Path:
        """Extract NAL units → temp .h264 → FFmpeg wrap → .mp4."""
        from .video import h264_to_mp4
        h264 = Path(out_mp4).with_suffix(".h264")
        self.export_h264(image, h264)
        h264_to_mp4(h264, out_mp4)
        return Path(out_mp4)
