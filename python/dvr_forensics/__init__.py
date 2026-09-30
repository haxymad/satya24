"""SATYA Python layer.

Two responsibilities, and only two:

1. `core` — thin subprocess wrapper around the Rust `satya` binary and
   the `satya-mcp` MCP server. All parsing, trust fusion, custody, and
   reporting live in Rust.

2. `ml` — ONNX-based inference (YOLOv8, ArcFace) and XAI (Grad-CAM) run
   on frames the Rust core extracts. Nothing else.
"""

__version__ = "0.1.0"

from .core import Satya, SatyaError

__all__ = ["Satya", "SatyaError", "__version__"]
