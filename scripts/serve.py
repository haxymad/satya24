#!/usr/bin/env python3
"""Serve a recovered MP4 in the browser."""
import sys
from pathlib import Path

from fastapi import FastAPI
from fastapi.responses import FileResponse, HTMLResponse
import uvicorn

app = FastAPI(title="SATYA Viewer")

VIDEO_PATH: Path = None

HTML = """
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>SATYA Viewer</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 0;
           background: #0e0e10; color: #e8e8e8; }
    header { padding: 16px 24px; border-bottom: 1px solid #222;
             display: flex; align-items: center; gap: 16px; }
    h1 { margin: 0; font-size: 18px; font-weight: 600; }
    .badge { background: #2a2a30; padding: 4px 10px; border-radius: 6px;
             font-size: 12px; color: #9a9aa5; }
    main { padding: 24px; max-width: 1000px; margin: 0 auto; }
    video { width: 100%; background: #000; border-radius: 8px; }
    .meta { margin-top: 16px; font-size: 13px; color: #8a8a95; }
  </style>
</head>
<body>
  <header>
    <h1>SATYA — Recovered Video</h1>
    <span class="badge">prototype</span>
  </header>
  <main>
    <video controls autoplay muted>
      <source src="/video" type="video/mp4">
    </video>
    <div class="meta"><p>Source: <code id="src"></code></p></div>
  </main>
  <script>
    fetch('/meta').then(r => r.json()).then(m => {
      document.getElementById('src').textContent = m.path;
    });
  </script>
</body>
</html>
"""


@app.get("/", response_class=HTMLResponse)
def index():
    return HTML


@app.get("/video")
def video():
    return FileResponse(VIDEO_PATH, media_type="video/mp4")


@app.get("/meta")
def meta():
    return {"path": str(VIDEO_PATH)}


def main():
    global VIDEO_PATH
    if len(sys.argv) != 2:
        sys.exit(f"Usage: {sys.argv[0]} <video.mp4>")
    VIDEO_PATH = Path(sys.argv[1]).resolve()
    if not VIDEO_PATH.exists():
        sys.exit(f"Not found: {VIDEO_PATH}")
    print(f"Serving {VIDEO_PATH}")
    print("Open http://127.0.0.1:8000")
    uvicorn.run(app, host="127.0.0.1", port=8000)


if __name__ == "__main__":
    main()
