"""ArcFace face recognition via ONNX."""
from __future__ import annotations

from pathlib import Path
from typing import Optional

import cv2
import numpy as np

DEFAULT_MODEL = Path("models/arcface.onnx")


class FaceRecognizer:
    def __init__(self, model_path: Optional[str | Path] = None):
        import onnxruntime as ort

        path = Path(model_path) if model_path else DEFAULT_MODEL
        if not path.exists():
            raise FileNotFoundError(
                f"ArcFace ONNX model not found: {path}\n"
                f"Obtain from insightface model zoo and rename to arcface.onnx"
            )
        self.session = ort.InferenceSession(
            str(path), providers=["CPUExecutionProvider"]
        )
        self.input_name = self.session.get_inputs()[0].name

    def embed(self, face_img: np.ndarray) -> np.ndarray:
        """face_img: 112x112x3 uint8 RGB. Returns 512-d L2-normalized embedding."""
        img = cv2.resize(face_img, (112, 112))
        x = (img.astype(np.float32) - 127.5) / 128.0
        x = x.transpose(2, 0, 1)[None, ...]
        emb = self.session.run(None, {self.input_name: x})[0][0]
        return emb / (np.linalg.norm(emb) + 1e-9)

    @staticmethod
    def cosine_sim(a: np.ndarray, b: np.ndarray) -> float:
        return float(np.dot(a, b) / (np.linalg.norm(a) * np.linalg.norm(b) + 1e-9))

    def match(self, a: np.ndarray, b: np.ndarray, threshold: float = 0.5) -> tuple[bool, float]:
        sim = self.cosine_sim(a, b)
        return sim >= threshold, sim
