"""Make a 2-D map of BirdNET V2.4's 6,522 classes from the model's own weights.

Each class's row in the final dense layer (1,024 weights) points the way a
window's embedding has to lean for that class to score high. Classes whose rows
point the same way sound alike to the model. t-SNE on those rows (cosine
distance) gives a map where related species sit together.

  python -I project.py MODEL.onnx LABELS.txt OUT_DIR

MODEL.onnx is birdnet-v2.4-embeddings-fp32.onnx (the file the app downloads).
Writes, in OUT_DIR:
  birdnet-map.bin   6,522 x 2 int16, little endian: x then y per class, in label
                    order, scaled to -32767..32767. 26 KB; the names come from
                    the label file the app already has.
  check.txt         how often a class's nearest neighbours share its genus, on
                    the map and in the 1,024-d weights, against random pairs.

Needs onnx, numpy, scikit-learn. Takes about 1 to 2 minutes.
"""

import os
import sys

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.manifold import TSNE
from sklearn.neighbors import NearestNeighbors

DENSE = "const_fold_opt__1508"  # MatMul initializer, 1024 x 6522
CLASSES = 6522
SEED = 1


def class_rows(model_path):
    model = onnx.load(model_path)
    for init in model.graph.initializer:
        if init.name == DENSE:
            w = numpy_helper.to_array(init).astype(np.float32)
            assert w.shape == (1024, CLASSES), w.shape
            rows = w.T
            return rows / np.linalg.norm(rows, axis=1, keepdims=True)
    raise SystemExit(f"{DENSE} not found in {model_path}")


def genus_share(points, genera, k, metric):
    """Share of each class's k nearest neighbours that have its genus."""
    nn = NearestNeighbors(n_neighbors=k + 1, metric=metric).fit(points)
    _, idx = nn.kneighbors(points)
    same = [np.mean([genera[j] == genera[i] for j in row[1:]]) for i, row in enumerate(idx)]
    return float(np.mean(same))


def main():
    model_path, labels_path, out_dir = sys.argv[1:4]
    os.makedirs(out_dir, exist_ok=True)
    labels = [l.rstrip("\n") for l in open(labels_path, encoding="utf-8") if l.strip()]
    assert len(labels) == CLASSES, len(labels)
    genera = [l.split("_")[0].split(" ")[0] for l in labels]

    rows = class_rows(model_path)
    xy = TSNE(
        n_components=2,
        metric="cosine",
        perplexity=30,
        init="pca",
        random_state=SEED,
    ).fit_transform(rows)

    xy = xy - xy.mean(axis=0)
    scaled = np.round(xy / np.abs(xy).max() * 32767).astype("<i2")
    scaled.tofile(os.path.join(out_dir, "birdnet-map.bin"))

    rng = np.random.default_rng(SEED)
    pairs = rng.integers(0, CLASSES, size=(20000, 2))
    random_share = float(np.mean([genera[a] == genera[b] for a, b in pairs if a != b]))
    lines = [
        f"5 nearest neighbours with the same genus, on the map: {genus_share(xy, genera, 5, 'euclidean'):.3f}",
        f"5 nearest neighbours with the same genus, 1,024-d weights: {genus_share(rows, genera, 5, 'cosine'):.3f}",
        f"Random pairs with the same genus: {random_share:.4f}",
    ]
    with open(os.path.join(out_dir, "check.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
