#!/usr/bin/env python3
"""
SATYA ML sidecar — full inference over HTTP.

Endpoints:
  GET  /health           service status
  POST /detect           YOLOv8 object detection on one frame
  POST /faces            RetinaFace detection + ArcFace embedding
  POST /motion           Motion detection (frame difference)
  POST /analyze-all      Batch: objects + faces + motion over sampled frames
  POST /cluster-faces    Cluster face embeddings
  POST /correlate        Cross-camera event correlation
  POST /thumbnail        Extract frame as base64 PNG

Run:
  pip install fastapi uvicorn onnxruntime opencv-python numpy scikit-learn
  python3 -m dvr_forensics.ml_server
"""
from __future__ import annotations

import base64
import os
import subprocess
import tempfile
from pathlib import Path
from typing import Optional

import cv2
import numpy as np
import onnxruntime as ort
from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

YOLO_MODEL = os.environ.get("SATYA_YOLO_MODEL", "models/yolov8n.onnx")
FACE_DETECT_MODEL = os.environ.get("SATYA_FACE_DETECT", "models/retinaface.onnx")
FACE_EMBED_MODEL = os.environ.get("SATYA_FACE_EMBED", "models/arcface.onnx")
FACE_MATCH_THRESHOLD = float(os.environ.get("SATYA_FACE_THRESHOLD", "0.5"))

COCO = {
    0:"person",1:"bicycle",2:"car",3:"motorcycle",4:"airplane",5:"bus",
    6:"train",7:"truck",8:"boat",9:"traffic light",10:"fire hydrant",
    11:"stop sign",12:"parking meter",13:"bench",14:"bird",15:"cat",
    16:"dog",17:"horse",18:"sheep",19:"cow",20:"elephant",21:"bear",
    22:"zebra",23:"giraffe",24:"backpack",25:"umbrella",26:"handbag",
    27:"tie",28:"suitcase",29:"frisbee",30:"skis",31:"snowboard",
    32:"sports ball",33:"kite",34:"baseball bat",35:"baseball glove",
    36:"skateboard",37:"surfboard",38:"tennis racket",39:"bottle",
    40:"wine glass",41:"cup",42:"fork",43:"knife",44:"spoon",45:"bowl",
    46:"banana",47:"apple",48:"sandwich",49:"orange",50:"broccoli",
    51:"carrot",52:"hot dog",53:"pizza",54:"donut",55:"cake",56:"chair",
    57:"couch",58:"potted plant",59:"bed",60:"dining table",61:"toilet",
    62:"tv",63:"laptop",64:"mouse",65:"remote",66:"keyboard",
    67:"cell phone",68:"microwave",69:"oven",70:"toaster",71:"sink",
    72:"refrigerator",73:"book",74:"clock",75:"vase",76:"scissors",
    77:"teddy bear",78:"hair drier",79:"toothbrush",
}

def encode_png_base64(bgr: np.ndarray, max_w: int = 200) -> str:
    """Encode a BGR image as base64 PNG, downscaled."""
    h, w = bgr.shape[:2]
    if w > max_w:
        nh = int(h * max_w / w)
        bgr = cv2.resize(bgr, (max_w, nh))
    ok, buf = cv2.imencode(".png", bgr)
    if not ok:
        return ""
    return base64.b64encode(buf.tobytes()).decode()


def crop_box(bgr: np.ndarray, box: list[int]) -> np.ndarray | None:
    x1, y1, x2, y2 = [int(v) for v in box]
    h, w = bgr.shape[:2]
    x1 = max(0, min(x1, w - 1))
    y1 = max(0, min(y1, h - 1))
    x2 = max(x1 + 1, min(x2, w))
    y2 = max(y1 + 1, min(y2, h))
    if x2 - x1 < 6 or y2 - y1 < 6:
        return None
    return bgr[y1:y2, x1:x2].copy()


app = FastAPI(title="SATYA ML Sidecar")
import traceback
from fastapi.responses import JSONResponse

@app.exception_handler(Exception)
async def _unhandled(request, exc):
    tb = traceback.format_exc()
    print("=== UNHANDLED EXCEPTION ===", flush=True)
    print(tb, flush=True)
    return JSONResponse(status_code=500, content={"error": str(exc), "traceback": tb})
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"], allow_methods=["*"], allow_headers=["*"],
)

_sessions: dict[str, ort.InferenceSession] = {}


def get_session(kind: str) -> Optional[ort.InferenceSession]:
    if kind in _sessions:
        return _sessions[kind]
    paths = {
        "yolo": YOLO_MODEL,
        "face_detect": FACE_DETECT_MODEL,
        "face_embed": FACE_EMBED_MODEL,
    }
    p = Path(paths.get(kind, ""))
    if not p.exists():
        return None
    _sessions[kind] = ort.InferenceSession(
        str(p), providers=["CPUExecutionProvider"]
    )
    return _sessions[kind]


class DetectRequest(BaseModel):
    video_path: str
    frame_index: int = 0
    conf: float = 0.35
    iou: float = 0.45


class MotionRequest(BaseModel):
    video_path: str
    start_frame: int = 0
    end_frame: int = 100
    step: int = 1
    threshold: float = 25.0


class AnalyzeAllRequest(BaseModel):
    video_path: str
    sample_every: int = 25
    run_objects: bool = True
    run_faces: bool = True
    run_motion: bool = True
    conf: float = 0.35


class ClusterRequest(BaseModel):
    embeddings: list[list[float]]
    threshold: float = 0.5


class CorrelateRequest(BaseModel):
    events_a: list[dict]
    events_b: list[dict]
    time_tolerance_s: float = 5.0


class ThumbnailRequest(BaseModel):
    video_path: str
    frame_index: int = 0
    width: int = 480


class Detection(BaseModel):
    label: str
    class_id: int
    score: float
    bbox: list[int]


class Face(BaseModel):
    bbox: list[int]
    score: float
    embedding: Optional[list[float]] = None


@app.get("/health")
def health():
    return {
        "ok": True,
        "models": {
            "yolo": Path(YOLO_MODEL).exists(),
            "face_detect": Path(FACE_DETECT_MODEL).exists(),
            "face_embed": Path(FACE_EMBED_MODEL).exists(),
        },
        "paths": {
            "yolo": YOLO_MODEL,
            "face_detect": FACE_DETECT_MODEL,
            "face_embed": FACE_EMBED_MODEL,
        },
    }


@app.post("/detect")
def detect(req: DetectRequest) -> dict:
    import traceback
    try:
        frame = extract_frame(req.video_path, req.frame_index)
    except Exception as e:
        tb = traceback.format_exc()
        print(f"extract_frame failed: {e}\n{tb}", flush=True)
        raise HTTPException(500, f"extract_frame: {e}")
    if frame is None:
        raise HTTPException(400, f"cannot read frame {req.frame_index} from {req.video_path}")
    session = get_session("yolo")
    if session is None:
        raise HTTPException(500, f"YOLO model missing at {YOLO_MODEL}")
    try:
        dets = run_yolo(session, frame, req.conf, req.iou)
    except Exception as e:
        tb = traceback.format_exc()
        print(f"run_yolo failed: {e}\n{tb}", flush=True)
        raise HTTPException(500, f"run_yolo: {e}")
    return {"frame_index": req.frame_index,
            "detections": [d.dict() for d in dets],
            "count": len(dets)}


@app.post("/faces")
def faces(req: DetectRequest) -> dict:
    frame = extract_frame(req.video_path, req.frame_index)
    if frame is None:
        raise HTTPException(400, f"cannot read frame {req.frame_index}")
    det = get_session("face_detect")
    emb = get_session("face_embed")
    if det is None:
        return {
            "frame_index": req.frame_index,
            "faces": [],
            "note": f"Face model not installed. Download RetinaFace ONNX to {FACE_DETECT_MODEL}",
        }
    boxes = run_face_detect(det, frame)
    out = []
    for box, score in boxes:
        crop = crop_face(frame, box)
        embedding = None
        if emb is not None and crop is not None:
            embedding = run_face_embed(emb, crop).tolist()
        out.append(Face(bbox=box, score=score, embedding=embedding).dict())
    return {"frame_index": req.frame_index, "faces": out, "count": len(out)}


@app.post("/motion")
def motion(req: MotionRequest) -> dict:
    cap = cv2.VideoCapture(req.video_path)
    if not cap.isOpened():
        raise HTTPException(400, f"cannot open {req.video_path}")
    cap.set(cv2.CAP_PROP_POS_FRAMES, req.start_frame)
    prev_gray = None
    events = []
    idx = req.start_frame
    while idx <= req.end_frame:
        ok, frame = cap.read()
        if not ok:
            break
        if (idx - req.start_frame) % req.step != 0:
            idx += 1
            continue
        gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
        gray = cv2.GaussianBlur(gray, (21, 21), 0)
        if prev_gray is not None:
            diff = cv2.absdiff(prev_gray, gray)
            _, thresh = cv2.threshold(diff, req.threshold, 255, cv2.THRESH_BINARY)
            pct = float(np.count_nonzero(thresh)) / thresh.size * 100.0
            contours, _ = cv2.findContours(thresh, cv2.RETR_EXTERNAL,
                                           cv2.CHAIN_APPROX_SIMPLE)
            regions = []
            for c in contours:
                if cv2.contourArea(c) < 500:
                    continue
                x, y, w, h = cv2.boundingRect(c)
                regions.append([int(x), int(y), int(w), int(h)])
            if pct > 0.5:
                events.append({"frame_index": idx,
                               "motion_pct": round(pct, 3),
                               "regions": regions[:10]})
        prev_gray = gray
        idx += 1
    cap.release()
    return {"start": req.start_frame, "end": req.end_frame,
            "step": req.step, "event_count": len(events),
            "events": events}


@app.post("/analyze-all")
def analyze_all(req: AnalyzeAllRequest) -> dict:
    cap = cv2.VideoCapture(req.video_path)
    if not cap.isOpened():
        raise HTTPException(400, f"cannot open {req.video_path}")
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
    fps = cap.get(cv2.CAP_PROP_FPS) or 25.0

    yolo = get_session("yolo") if req.run_objects else None
    face_det = get_session("face_detect") if req.run_faces else None
    face_emb = get_session("face_embed") if req.run_faces else None

    prev_gray = None
    out = []
    all_object_counts: dict[str, int] = {}
    total_faces = 0

    for idx in range(0, total, req.sample_every):
        cap.set(cv2.CAP_PROP_POS_FRAMES, idx)
        ok, frame = cap.read()
        if not ok:
            break

        entry = {
            "frame_index": idx,
            "timestamp_s": round(idx / fps, 3) if fps else None,
            "objects": [],
            "faces": [],
            "motion_pct": None,
        }

        if yolo is not None:
            dets = run_yolo(yolo, frame, req.conf, 0.45)
            objs = []
            for d in dets:
                o = d.dict()
                # Embed a small PNG crop for every detection
                try:
                    crop = crop_box(frame, d.bbox)
                    if crop is not None:
                        o["crop_png_b64"] = encode_png_base64(crop, max_w=180)
                except Exception:
                    pass
                objs.append(o)
                all_object_counts[d.label] = all_object_counts.get(d.label, 0) + 1
            entry["objects"] = objs

        if face_det is not None:
            boxes = run_face_detect(face_det, frame)
            for box, score in boxes:
                f = {"bbox": box, "score": score}
                if face_emb is not None:
                    crop = crop_face(frame, box)
                    if crop is not None:
                        f["embedding"] = run_face_embed(face_emb, crop).tolist()
                entry["faces"].append(f)
                total_faces += 1

        if req.run_motion:
            gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
            gray = cv2.GaussianBlur(gray, (21, 21), 0)
            if prev_gray is not None:
                diff = cv2.absdiff(prev_gray, gray)
                _, thresh = cv2.threshold(diff, 25, 255, cv2.THRESH_BINARY)
                pct = float(np.count_nonzero(thresh)) / thresh.size * 100.0
                entry["motion_pct"] = round(pct, 3)
            prev_gray = gray

        out.append(entry)

    cap.release()
    return {
        "video_path": req.video_path,
        "total_frames": total,
        "sampled": len(out),
        "sample_every": req.sample_every,
        "summary": {
            "total_objects": sum(all_object_counts.values()),
            "object_breakdown": all_object_counts,
            "total_faces": total_faces,
        },
        "frames": out,
    }


@app.post("/cluster-faces")
def cluster_faces(req: ClusterRequest) -> dict:
    if not req.embeddings:
        return {"clusters": [], "count": 0}
    from sklearn.cluster import DBSCAN
    X = np.asarray(req.embeddings, dtype=np.float32)
    norms = np.linalg.norm(X, axis=1, keepdims=True) + 1e-9
    X = X / norms
    labels = DBSCAN(eps=req.threshold, min_samples=2, metric="cosine").fit_predict(X)
    clusters: dict[int, list[int]] = {}
    for i, lab in enumerate(labels):
        if lab == -1:
            continue
        clusters.setdefault(int(lab), []).append(i)
    return {
        "count": len(clusters),
        "clusters": [
            {"cluster_id": cid, "indices": idxs, "size": len(idxs)}
            for cid, idxs in clusters.items()
        ],
    }


@app.post("/correlate")
def correlate(req: CorrelateRequest) -> dict:
    """Cross-camera event correlation: find pairs of events within tolerance."""
    matches = []
    for a in req.events_a:
        ta = a.get("timestamp_s")
        if ta is None:
            continue
        for b in req.events_b:
            tb = b.get("timestamp_s")
            if tb is None:
                continue
            delta = abs(ta - tb)
            if delta <= req.time_tolerance_s:
                matches.append({
                    "a": a,
                    "b": b,
                    "delta_s": round(delta, 3),
                })
    matches.sort(key=lambda m: m["delta_s"])
    return {"match_count": len(matches), "matches": matches[:500]}


@app.post("/thumbnail")
def thumbnail(req: ThumbnailRequest) -> dict:
    frame = extract_frame(req.video_path, req.frame_index)
    if frame is None:
        raise HTTPException(400, f"cannot read frame {req.frame_index}")
    h, w = frame.shape[:2]
    if req.width and req.width < w:
        new_h = int(h * req.width / w)
        frame = cv2.resize(frame, (req.width, new_h))
    ok, buf = cv2.imencode(".png", frame)
    if not ok:
        raise HTTPException(500, "png encode failed")
    return {
        "frame_index": req.frame_index,
        "width": int(frame.shape[1]),
        "height": int(frame.shape[0]),
        "png_base64": base64.b64encode(buf.tobytes()).decode(),
    }


# ---------------------------------------------------------------------------
# Inference primitives
# ---------------------------------------------------------------------------

def extract_frame(video: str, index: int) -> Optional[np.ndarray]:
    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as tmp:
        out = tmp.name
    try:
        cmd = ["ffmpeg", "-y", "-i", video,
               "-vf", f"select=eq(n\\,{index})",
               "-vframes", "1", out]
        r = subprocess.run(cmd, capture_output=True, timeout=30)
        if r.returncode != 0 or not Path(out).exists():
            return None
        return cv2.imread(out)
    finally:
        Path(out).unlink(missing_ok=True)


def run_yolo(session, bgr: np.ndarray, conf_thr: float, iou_thr: float) -> list[Detection]:
    h, w = bgr.shape[:2]
    img = cv2.resize(bgr, (640, 640))
    img = img[:, :, ::-1].transpose(2, 0, 1).astype(np.float32) / 255.0
    blob = img[None, ...]
    out = session.run(None, {session.get_inputs()[0].name: blob})[0]

    # Handle both layouts:
    #   (1, 84, 8400) → transposed to (8400, 84)  [standard ultralytics]
    #   (1, 8400, 84) → already in (8400, 84)
    if out.ndim != 3:
        raise RuntimeError(f"unexpected YOLO output rank {out.ndim}")

    a, b = out.shape[1], out.shape[2]
    if a < b:
        # (1, 84, 8400) — transpose
        pred = out[0].T
    else:
        # (1, 8400, 84) — already correct
        pred = out[0]

    if pred.shape[1] < 5:
        raise RuntimeError(f"YOLO output has {pred.shape[1]} columns, expected >= 5")

    boxes_xywh = pred[:, :4]
    scores = pred[:, 4:]
    cids = scores.argmax(axis=1)
    confs = scores[np.arange(len(scores)), cids]
    keep = confs > conf_thr
    boxes_xywh = boxes_xywh[keep]
    confs = confs[keep]
    cids = cids[keep]
    if len(boxes_xywh) == 0:
        return []

    boxes = np.empty_like(boxes_xywh)
    boxes[:, 0] = boxes_xywh[:, 0] - boxes_xywh[:, 2] / 2
    boxes[:, 1] = boxes_xywh[:, 1] - boxes_xywh[:, 3] / 2
    boxes[:, 2] = boxes_xywh[:, 0] + boxes_xywh[:, 2] / 2
    boxes[:, 3] = boxes_xywh[:, 1] + boxes_xywh[:, 3] / 2
    boxes[:, [0, 2]] *= w / 640
    boxes[:, [1, 3]] *= h / 640
    idx = nms(boxes, confs, iou_thr)
    return [Detection(
        label=COCO.get(int(cids[i]), f"class_{cids[i]}"),
        class_id=int(cids[i]),
        score=float(confs[i]),
        bbox=[int(v) for v in boxes[i]],
    ) for i in idx]


def run_face_detect(session, bgr: np.ndarray) -> list[tuple[list[int], float]]:
    """RetinaFace-style detection. Returns list of (bbox_xyxy, score)."""
    h, w = bgr.shape[:2]
    img = cv2.resize(bgr, (640, 640)).astype(np.float32)
    img = (img - 127.5) / 128.0
    blob = img.transpose(2, 0, 1)[None, ...]
    try:
        out = session.run(None, {session.get_inputs()[0].name: blob})
    except Exception:
        return []
    if not out or out[0] is None:
        return []
    dets = out[0]
    if dets.ndim == 3:
        dets = dets[0]
    results = []
    for row in dets:
        if len(row) < 5:
            continue
        score = float(row[4])
        if score < 0.5:
            continue
        x1, y1, x2, y2 = [float(v) for v in row[:4]]
        x1 = int(x1 * w / 640)
        y1 = int(y1 * h / 640)
        x2 = int(x2 * w / 640)
        y2 = int(y2 * h / 640)
        results.append(([x1, y1, x2, y2], score))
    return results


def run_face_embed(session, bgr_crop: np.ndarray) -> np.ndarray:
    img = cv2.resize(bgr_crop, (112, 112)).astype(np.float32)
    img = img[:, :, ::-1]
    img = (img - 127.5) / 128.0
    blob = img.transpose(2, 0, 1)[None, ...]
    out = session.run(None, {session.get_inputs()[0].name: blob})[0][0]
    return out / (np.linalg.norm(out) + 1e-9)


def crop_face(bgr: np.ndarray, box: list[int]) -> Optional[np.ndarray]:
    x1, y1, x2, y2 = box
    h, w = bgr.shape[:2]
    x1 = max(0, min(x1, w - 1))
    y1 = max(0, min(y1, h - 1))
    x2 = max(x1 + 1, min(x2, w))
    y2 = max(y1 + 1, min(y2, h))
    if x2 - x1 < 4 or y2 - y1 < 4:
        return None
    return bgr[y1:y2, x1:x2]


def nms(boxes: np.ndarray, scores: np.ndarray, iou_thr: float) -> list[int]:
    x1, y1, x2, y2 = boxes.T
    areas = (x2 - x1) * (y2 - y1)
    order = scores.argsort()[::-1]
    keep = []
    while order.size > 0:
        i = order[0]
        keep.append(int(i))
        xx1 = np.maximum(x1[i], x1[order[1:]])
        yy1 = np.maximum(y1[i], y1[order[1:]])
        xx2 = np.minimum(x2[i], x2[order[1:]])
        yy2 = np.minimum(y2[i], y2[order[1:]])
        inter = np.maximum(0.0, xx2 - xx1) * np.maximum(0.0, yy2 - yy1)
        iou = inter / (areas[i] + areas[order[1:]] - inter + 1e-9)
        order = order[1:][iou <= iou_thr]
    return keep


if __name__ == "__main__":
    import uvicorn, os as _os
    print("SATYA ML sidecar starting on :8001")
    print(f"YOLO:        {YOLO_MODEL} (exists={Path(YOLO_MODEL).exists()})")
    print(f"Face detect: {FACE_DETECT_MODEL} (exists={Path(FACE_DETECT_MODEL).exists()})")
    print(f"Face embed:  {FACE_EMBED_MODEL} (exists={Path(FACE_EMBED_MODEL).exists()})")
    _bind = _os.environ.get("SATYA_ML_BIND", "127.0.0.1:8001")
    _host, _, _port = _bind.partition(":")
    uvicorn.run(app, host=_host, port=int(_port or 8001))
