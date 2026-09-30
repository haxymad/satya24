"""YOLOv8 object detection via ONNX Runtime (CPU)."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import cv2
import numpy as np

DEFAULT_MODEL = Path("models/yolov8n.onnx")


@dataclass
class Detection:
    bbox: tuple[int, int, int, int]   # x1, y1, x2, y2 (pixel coords)
    class_id: int
    score: float
    label: str = ""


class ObjectDetector:
    def __init__(
        self,
        model_path: Optional[str | Path] = None,
        *,
        conf_threshold: float = 0.35,
        iou_threshold: float = 0.45,
    ):
        import onnxruntime as ort

        path = Path(model_path) if model_path else DEFAULT_MODEL
        if not path.exists():
            raise FileNotFoundError(
                f"YOLOv8 ONNX model not found: {path}\n"
                f"Download the nano model:\n"
                f"  pip install ultralytics\n"
                f"  yolo export model=yolov8n.pt format=onnx"
            )
        self.session = ort.InferenceSession(
            str(path), providers=["CPUExecutionProvider"]
        )
        self.input_name = self.session.get_inputs()[0].name
        self.conf_threshold = conf_threshold
        self.iou_threshold = iou_threshold
        self._labels = self._coco_labels()

    def detect(self, frame: np.ndarray) -> list[Detection]:
        """frame: HxWx3 uint8 RGB. Returns list of Detection."""
        h, w = frame.shape[:2]
        blob = self._preprocess(frame)
        out = self.session.run(None, {self.input_name: blob})[0]
        boxes, scores, class_ids = self._postprocess(out, w, h)

        detections: list[Detection] = []
        for box, score, cid in zip(boxes, scores, class_ids):
            label = self._labels.get(int(cid), f"class_{cid}")
            detections.append(Detection(
                bbox=tuple(int(v) for v in box),
                class_id=int(cid),
                score=float(score),
                label=label,
            ))
        return detections

    # -- internals ----------------------------------------------------------

    def _preprocess(self, frame: np.ndarray) -> np.ndarray:
        img = cv2.resize(frame, (640, 640))
        img = img[:, :, ::-1]                       # RGB → BGR
        img = img.transpose(2, 0, 1).astype(np.float32) / 255.0
        return img[None, ...]

    def _postprocess(self, out: np.ndarray, w: int, h: int):
        # YOLOv8 output shape: (1, 84, 8400) → transpose to (8400, 84)
        pred = out[0].T
        boxes_xywh = pred[:, :4]
        scores = pred[:, 4:]
        class_ids = scores.argmax(axis=1)
        confs = scores[np.arange(len(scores)), class_ids]

        keep = confs > self.conf_threshold
        boxes_xywh = boxes_xywh[keep]
        confs = confs[keep]
        class_ids = class_ids[keep]

        if len(boxes_xywh) == 0:
            return [], [], []

        boxes_xyxy = np.empty_like(boxes_xywh)
        boxes_xyxy[:, 0] = boxes_xywh[:, 0] - boxes_xywh[:, 2] / 2
        boxes_xyxy[:, 1] = boxes_xywh[:, 1] - boxes_xywh[:, 3] / 2
        boxes_xyxy[:, 2] = boxes_xywh[:, 0] + boxes_xywh[:, 2] / 2
        boxes_xyxy[:, 3] = boxes_xywh[:, 1] + boxes_xywh[:, 3] / 2

        # Scale back to original frame size
        boxes_xyxy[:, [0, 2]] *= w / 640
        boxes_xyxy[:, [1, 3]] *= h / 640

        indices = self._nms(boxes_xyxy, confs)
        return boxes_xyxy[indices], confs[indices], class_ids[indices]

    def _nms(self, boxes: np.ndarray, scores: np.ndarray) -> list[int]:
        x1, y1, x2, y2 = boxes.T
        areas = (x2 - x1) * (y2 - y1)
        order = scores.argsort()[::-1]
        keep: list[int] = []
        while order.size > 0:
            i = order[0]
            keep.append(int(i))
            xx1 = np.maximum(x1[i], x1[order[1:]])
            yy1 = np.maximum(y1[i], y1[order[1:]])
            xx2 = np.minimum(x2[i], x2[order[1:]])
            yy2 = np.minimum(y2[i], y2[order[1:]])
            inter = np.maximum(0.0, xx2 - xx1) * np.maximum(0.0, yy2 - yy1)
            iou = inter / (areas[i] + areas[order[1:]] - inter + 1e-9)
            order = order[1:][iou <= self.iou_threshold]
        return keep

    @staticmethod
    def _coco_labels() -> dict[int, str]:
        return {
            0: "person", 1: "bicycle", 2: "car", 3: "motorcycle", 4: "airplane",
            5: "bus", 6: "train", 7: "truck", 8: "boat", 9: "traffic light",
            10: "fire hydrant", 11: "stop sign", 12: "parking meter",
            13: "bench", 14: "bird", 15: "cat", 16: "dog", 17: "horse",
            18: "sheep", 19: "cow", 20: "elephant", 21: "bear", 22: "zebra",
            23: "giraffe", 24: "backpack", 25: "umbrella", 26: "handbag",
            27: "tie", 28: "suitcase", 29: "frisbee", 30: "skis",
            31: "snowboard", 32: "sports ball", 33: "kite", 34: "baseball bat",
            35: "baseball glove", 36: "skateboard", 37: "surfboard",
            38: "tennis racket", 39: "bottle", 40: "wine glass", 41: "cup",
            42: "fork", 43: "knife", 44: "spoon", 45: "bowl", 46: "banana",
            47: "apple", 48: "sandwich", 49: "orange", 50: "broccoli",
            51: "carrot", 52: "hot dog", 53: "pizza", 54: "donut", 55: "cake",
            56: "chair", 57: "couch", 58: "potted plant", 59: "bed",
            60: "dining table", 61: "toilet", 62: "tv", 63: "laptop",
            64: "mouse", 65: "remote", 66: "keyboard", 67: "cell phone",
            68: "microwave", 69: "oven", 70: "toaster", 71: "sink",
            72: "refrigerator", 73: "book", 74: "clock", 75: "vase",
            76: "scissors", 77: "teddy bear", 78: "hair drier",
            79: "toothbrush",
        }
