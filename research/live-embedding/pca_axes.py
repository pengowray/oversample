"""Can BirdNET V2.4's embedding be shown as a few named 1-D "slider" axes?

Fits PCA on the 6,522 class rows of the final dense layer (the same rows the
species map uses, research/species-map/project.py), then lists, for each of
the first components, the classes at its two ends and how much of the rows'
variance it explains. An axis is worth a slider only if its ends name
something a user would recognise.

  python -I pca_axes.py MODEL.onnx LABELS.txt [N_COMPONENTS]

Needs onnx, numpy, scikit-learn.
"""

import os
import sys

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.decomposition import PCA

DENSE = "const_fold_opt__1508"  # MatMul initializer, 1024 x 6522
CLASSES = 6522


def class_rows(model_path):
    model = onnx.load(model_path)
    for init in model.graph.initializer:
        if init.name == DENSE:
            w = numpy_helper.to_array(init).astype(np.float32)
            assert w.shape == (1024, CLASSES), w.shape
            return w.T
    raise SystemExit(f"{DENSE} not found in {model_path}")


def main():
    model, labels = sys.argv[1], sys.argv[2]
    n = int(sys.argv[3]) if len(sys.argv) > 3 else 6
    rows = class_rows(model)
    rows /= np.linalg.norm(rows, axis=1, keepdims=True)
    names = [l.strip().split("_", 1)[-1] for l in open(labels, encoding="utf-8")]
    pca = PCA(n_components=n, random_state=1).fit(rows)
    proj = pca.transform(rows)
    for k in range(n):
        order = np.argsort(proj[:, k])
        low = ", ".join(names[i] for i in order[:8])
        high = ", ".join(names[i] for i in order[-8:][::-1])
        print(f"PC{k + 1} ({pca.explained_variance_ratio_[k] * 100:.1f}% of variance)")
        print(f"  low:  {low}")
        print(f"  high: {high}")
    print(f"first {n} together: {pca.explained_variance_ratio_.sum() * 100:.1f}%")


if __name__ == "__main__":
    main()
