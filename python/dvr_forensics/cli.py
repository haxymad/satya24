"""SATYA Python CLI — ML and video helpers around the Rust core."""
from __future__ import annotations

from pathlib import Path
from typing import Optional

import typer
from rich.console import Console

from .core import Satya, SatyaError
from .video import h264_to_mp4, VideoError

app = typer.Typer(
    name="satya-py",
    help="SATYA Python helpers: ML inference and video export.",
    no_args_is_help=True,
)
console = Console()


@app.command()
def info(image: Path = typer.Option(..., "-i", "--image", exists=True)):
    """Show OEM and frame counts for a DVR image."""
    s = Satya()
    ident = s.identify(image)
    console.print(f"[bold]OEM:[/bold] {ident.get('oem', 'unknown')}")
    console.print(f"[bold]Confidence:[/bold] {ident.get('confidence', '?')}")

    frames = s.enumerate(image)
    allocated = sum(1 for f in frames if f.recovery_source == "Allocated")
    carved = len(frames) - allocated
    console.print(f"[bold]Frames:[/bold] {len(frames)} ({allocated} allocated, {carved} carved)")


@app.command()
def export(
    image: Path = typer.Option(..., "-i", "--image", exists=True),
    output: Path = typer.Option(..., "-o", "--output"),
):
    """Extract and wrap video as MP4."""
    s = Satya()
    try:
        mp4 = s.export_mp4(image, output)
        console.print(f"[green]Wrote {mp4} ({mp4.stat().st_size:,} bytes)[/green]")
    except (SatyaError, VideoError) as e:
        console.print(f"[red]{e}[/red]")
        raise typer.Exit(1)


@app.command()
def detect(
    image: Path = typer.Option(..., "-i", "--image", exists=True),
    model: Optional[Path] = typer.Option(None, "--model"),
    frame: int = typer.Option(0, "--frame", help="Frame index to sample"),
):
    """Run YOLOv8 on a sampled frame from the DVR image."""
    import cv2
    import numpy as np
    from .ml import ObjectDetector

    s = Satya()
    mp4 = image.with_suffix(".mp4")
    try:
        s.export_mp4(image, mp4)
    except (SatyaError, VideoError) as e:
        console.print(f"[red]{e}[/red]")
        raise typer.Exit(1)

    cap = cv2.VideoCapture(str(mp4))
    cap.set(cv2.CAP_PROP_POS_FRAMES, frame)
    ok, bgr = cap.read()
    cap.release()
    if not ok:
        console.print(f"[red]Could not read frame {frame} from {mp4}[/red]")
        raise typer.Exit(1)

    rgb = bgr[:, :, ::-1].copy()
    det = ObjectDetector(model)
    detections = det.detect(rgb)
    console.print(f"Detected {len(detections)} object(s):")
    for d in detections:
        console.print(f"  {d.label:<12} score={d.score:.2f}  bbox={d.bbox}")


@app.command()
def serve_mcp():
    """Placeholder — the MCP server is the Rust binary `satya-mcp`."""
    console.print(
        "The MCP server is the Rust binary. Run it directly:\n"
        "  ./target/release/satya-mcp\n\n"
        "Or register it with Claude Desktop / Cursor (see docs)."
    )


if __name__ == "__main__":
    app()
