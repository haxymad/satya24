"""Grad-CAM heatmaps for court-admissible AI evidence."""
from typing import Optional
import numpy as np


class GradCAM:
    def __init__(self, model_path: Optional[str] = None):
        # Framework-agnostic — works with any torchvision / timm model
        import torch
        self.torch = torch

    def explain(self, frame: np.ndarray, target_class: int) -> np.ndarray:
        """Return a HxW heatmap in [0,1] highlighting pixels that
        contributed to the target class prediction."""
        # Placeholder — real implementation attaches hooks to conv layers
        raise NotImplementedError("Wire to your model's last conv layer")
