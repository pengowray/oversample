"""Make a 2-D map of BirdNET+ V3.0's 11,725 classes from the model's own
weights, the V3.0 version of project.py.

Which weights. The pruned file the app runs
(BirdNET+_V3.0_Global_12K_FP16_pruned.onnx) has one head, the GeM head of the
full model: a single Gemm on the 1,280-value `embeddings` output,

    predictions = sigmoid(5 * (embeddings @ W.T + b) / s)

with W = `head.weight_stored_fp16` (11,725 x 1,280, float16), b =
`head.bias` and s = `scalar_tensor_default`. (The 5 and s come from the full
model's head sum, which the pruned file keeps with one head.) So as for V2.4,
each class's row of W points the way a window's embedding has to lean for
that class to score high. The script checks the formula against
onnxruntime on one window before using the rows.

  python -I project_v3.py MODEL.onnx LABELS.tsv OUT_DIR

MODEL.onnx is the pruned file; LABELS.tsv is
research/birdnet-v3-bench/birdnet-v3-labels.tsv (birdnet_id, scientific,
English name, taxon group). Writes, in OUT_DIR:

  birdnet-v3-map.bin   11,725 x 2 int16, little endian: x then y per class,
                       in label order, scaled to -32767..32767 (as
                       birdnet-map.bin).
  v3-check.txt         genus and taxon-group shares of each class's nearest
                       neighbours for a few projections, and the distance to
                       the 3rd nearest class (for the placement radius).
  v3-map.png           the map coloured by taxon group.

Needs onnx, onnxruntime, numpy, scikit-learn, matplotlib (and umap-js in node
for the UMAP rows of the check: ../live-embedding/umap_js.cjs).
"""

import os
import re
import subprocess
import sys
import tempfile
import time

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.decomposition import PCA
from sklearn.manifold import TSNE
from sklearn.neighbors import NearestNeighbors

HERE = os.path.dirname(os.path.abspath(__file__))
CLASSES = 11725
DIM = 1280
SEED = 1
# Taxon groups as the PNG and the check group them.
GROUPS = {"Aves": "birds", "Insecta": "insects", "Amphibia": "amphibians", "Mammalia": "mammals"}
SOUNDS = {"Anthropogenic", "Geophony", "Music"}
BINOMIAL = re.compile(r"^[A-Z][a-z]+ [a-z-]+")


def head(model_path):
    """W (classes x 1,280), b, and the divisor s, from the pruned file."""
    inits = {i.name: i for i in onnx.load(model_path).graph.initializer}
    w = numpy_helper.to_array(inits["head.weight_stored_fp16"]).astype(np.float32)
    b = numpy_helper.to_array(inits["head.bias"]).astype(np.float32)
    s = float(numpy_helper.to_array(inits["scalar_tensor_default"]))
    assert w.shape == (CLASSES, DIM), w.shape
    return w, b, s


def check_head(model_path, w, b, s):
    """The head formula against onnxruntime on one window of noise."""
    import onnxruntime as ort

    rng = np.random.default_rng(SEED)
    x = (rng.standard_normal((1, 96000)) * 0.05).astype(np.float32)
    sess = ort.InferenceSession(model_path, providers=["CPUExecutionProvider"])
    pred, emb = sess.run(["predictions", "embeddings"], {"input": x})
    mine = 1 / (1 + np.exp(-5 * (emb @ w.T + b) / s))
    return float(np.abs(mine - pred).max())


def read_labels(path):
    rows = [l.rstrip("\n").split("\t") for l in open(path, encoding="utf-8")]
    assert len(rows) == CLASSES, len(rows)
    return rows


def group_of(taxon):
    if taxon in GROUPS:
        return GROUPS[taxon]
    return "sounds" if taxon in SOUNDS else "other animals"


def share(points, keys, k, metric, only=None):
    """Share of each class's k nearest neighbours with the same key, over
    the classes in `only` (all when None)."""
    nn = NearestNeighbors(n_neighbors=k + 1, metric=metric).fit(points)
    _, idx = nn.kneighbors(points)
    rows = range(len(points)) if only is None else only
    return float(np.mean([np.mean([keys[j] == keys[i] for j in idx[i, 1:]]) for i in rows]))


def nth_neighbour(xy, n):
    d, _ = NearestNeighbors(n_neighbors=n + 1).fit(xy).kneighbors(xy)
    return d[:, n]


def umap_js(x, nn, md):
    with tempfile.TemporaryDirectory() as d:
        src, dst = os.path.join(d, "in.f32"), os.path.join(d, "out.f32")
        x.astype("<f4").tofile(src)
        script = os.path.join(HERE, "../live-embedding/umap_js.cjs")
        cmd = ["node", script, src, str(len(x)), str(x.shape[1]), dst, str(nn), str(md), str(SEED)]
        subprocess.run(cmd, check=True, capture_output=True, text=True)
        return np.fromfile(dst, "<f4").reshape(-1, 2)


def unit(x):
    return x / np.linalg.norm(x, axis=1, keepdims=True)


def plot(xy, groups, path, title):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    colours = {
        "birds": "#9aa3ad",
        "insects": "#e8843c",
        "amphibians": "#2e9d5b",
        "mammals": "#4a8ee0",
        "other animals": "#b04fc4",
        "sounds": "#111111",
    }
    fig, ax = plt.subplots(figsize=(9, 9), dpi=110)
    g = np.array(groups)
    for name, c in colours.items():
        sel = g == name
        size = 2 if name == "birds" else 5 if name != "sounds" else 18
        ax.scatter(xy[sel, 0], xy[sel, 1], s=size, c=c, lw=0, label=f"{name} ({sel.sum():,})")
    ax.set_aspect("equal")
    ax.set_xlim(-1.05, 1.05)
    ax.set_ylim(-1.05, 1.05)
    ax.set_title(title)
    ax.legend(loc="lower left", markerscale=3, fontsize=8)
    fig.tight_layout()
    fig.savefig(path)
    plt.close(fig)


def scale(xy):
    """Centre on the mean and scale so the furthest coordinate is 1 (as
    project.py does)."""
    xy = xy - xy.mean(axis=0)
    return xy / np.abs(xy).max()


def main():
    model_path, labels_path, out_dir = sys.argv[1:4]
    os.makedirs(out_dir, exist_ok=True)
    labels = read_labels(labels_path)
    sci = [r[1] for r in labels]
    taxon = [r[3] for r in labels]
    genera = [s.split(" ")[0] for s in sci]
    groups = [group_of(t) for t in taxon]
    species = [i for i in range(CLASSES) if BINOMIAL.match(sci[i]) and taxon[i] not in SOUNDS]

    w, b, s = head(model_path)
    norms = np.linalg.norm(w, axis=1)
    lines = [
        f"BirdNET+ V3.0 (pruned head), {CLASSES} classes. Class vector: the class's row of head.weight, scaled to length 1, cosine.",
        f"Head: predictions = sigmoid(5 * (embeddings @ W.T + b) / s), s = {s}; largest difference from onnxruntime on one window: {check_head(model_path, w, b, s):.1e}",
        f"Row lengths: min {norms.min():.3f}, median {np.median(norms):.3f}, max {norms.max():.3f}; rows of length 0: {(norms == 0).sum()}",
        f"Share of the 5 nearest neighbours that match: genus (over {len(species)} species), taxon group (birds, insects, amphibians, mammals, other animals, sounds; all classes).",
        "",
        f"{'method':32s}  genus  group   secs",
    ]
    rows = unit(w)

    rng = np.random.default_rng(SEED)
    pairs = rng.integers(0, CLASSES, size=(50000, 2))
    sp = np.array(species)
    sp_pairs = sp[rng.integers(0, len(sp), size=(50000, 2))]
    rand_genus = np.mean([genera[a] == genera[c] for a, c in sp_pairs if a != c])
    rand_group = np.mean([groups[a] == groups[c] for a, c in pairs if a != c])
    lines.append(f"{'random pairs':32s} {rand_genus:6.4f} {rand_group:6.3f}")

    def measure(name, xy, metric, secs):
        g = share(xy, genera, 5, metric, only=species)
        t = share(xy, groups, 5, metric)
        lines.append(f"{name:32s} {g:6.3f} {t:6.3f} {secs:6.0f}")
        print(lines[-1], flush=True)

    measure("full 1,280-d vectors", rows, "cosine", 0)
    pca50 = PCA(n_components=50, random_state=SEED).fit_transform(rows)
    measure("PCA to 50-d", pca50, "euclidean", 0)
    measure("PCA to 2-d", pca50[:, :2], "euclidean", 0)

    t = time.time()
    tsne = TSNE(n_components=2, metric="cosine", perplexity=30, init="pca", random_state=SEED).fit_transform(rows)
    measure("t-SNE p30 (full)", tsne, "euclidean", time.time() - t)
    t = time.time()
    um = umap_js(unit(pca50), 15, 0.1)
    measure("UMAP n15 (PCA-50, umap-js)", um, "euclidean", time.time() - t)

    xy = scale(tsne)
    np.round(xy * 32767).astype("<i2").tofile(os.path.join(out_dir, "birdnet-v3-map.bin"))
    d3 = nth_neighbour(xy, 3)
    lines += [
        "",
        "Chosen: t-SNE p30 (full), as for BirdNET V2.4 and Perch.",
        f"Distance to 3rd nearest class, map units: median {np.median(d3):.4f}, 90th percentile {np.percentile(d3, 90):.4f}",
        f"(BirdNET V2.4's map: median 0.0149, radius 0.15; so the V3.0 radius is 0.15 x {np.median(d3):.4f} / 0.0149 = {0.15 * np.median(d3) / 0.0149:.3f})",
    ]
    with open(os.path.join(out_dir, "v3-check.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    plot(xy, groups, os.path.join(out_dir, "v3-map.png"), "BirdNET+ V3.0 classes, t-SNE of the head's rows")


if __name__ == "__main__":
    main()
