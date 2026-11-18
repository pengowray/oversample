"""Data for the live view's component maps (birdnet-pc.bin).

Fits the same PCA as pca_axes.py on BirdNET V2.4's 6,522 class rows (the
final dense layer, each row scaled to length 1, as the species map uses), and
writes, for the first N components:

  - each component's share of the rows' variance,
  - the component's axis (1,024 values), so the app can project a live
    embedding onto it,
  - the percentile rank of every class's score on it (one byte per class),
  - 21 reference quantiles (0, 5, ..., 100 %) of real recordings' embeddings
    projected onto it.

The live embedding and the class rows share the 1,024 dimensions (a class's
logit is the embedding times its row, plus a bias), but not their spread: the
embedding is non-negative and sits off to one side of the rows on most axes.
Compared directly, a reading lands in the top or bottom tenth of species
nearly always. So the app takes the reading's percentile among the reference
recordings and highlights the species at that same percentile of the class
rows (quantile mapping).

  python -I pc_bands.py MODEL.onnx LABELS.txt OUT.bin EMB.npz [EMB.npz ...]

EMB.npz files hold BirdNET embeddings of real recordings under key "birdnet"
(shape [..., 1024]); the ones used were BirdSet's HSN and POW test sets
(batchi-noisier2noise/birdset_embed.py, raw arm). With LABELS.npz beside each
(rows in the same order), it also prints a check: how close the reading's
percentile is to its top class's percentile, against shuffled pairs.

Needs onnx, numpy, scikit-learn.

File layout (little endian):
  u8[4]  "OPC1"
  u16    components N, u16 dimensions D (1024), u16 classes C, u16 quantiles Q (21)
  f32[N]       share of variance
  f32[N][D]    axes
  f32[N][Q]    reference quantiles of (embedding / |embedding|) . axis
  u8[N][C]     class percentile rank on each axis, 0 to 255
"""

import os
import struct
import sys

import numpy as np
import onnx
from onnx import numpy_helper
from sklearn.decomposition import PCA

DENSE = "const_fold_opt__1508"  # MatMul initializer, 1024 x 6522
BIAS = "model/CLASS_DENSE_LAYER/BiasAdd/ReadVariableOp/resource"
CLASSES = 6522
N = 8
QUANTILES = np.linspace(0, 100, 21)


def main():
    model_path, labels_path, out_path, *embs = sys.argv[1:]
    inits = {i.name: i for i in onnx.load(model_path).graph.initializer}
    w = numpy_helper.to_array(inits[DENSE]).astype(np.float32)
    bias = numpy_helper.to_array(inits[BIAS]).astype(np.float32)
    assert w.shape == (1024, CLASSES), w.shape
    rows = w.T / np.linalg.norm(w.T, axis=1, keepdims=True)
    names = [l.strip().split("_", 1)[-1] for l in open(labels_path, encoding="utf-8")]

    pca = PCA(n_components=N, random_state=1).fit(rows)
    axes = pca.components_.astype(np.float32)
    score = rows @ axes.T
    order = np.argsort(score, axis=0)
    rank = np.empty_like(order)
    for k in range(N):
        rank[order[:, k], k] = np.arange(CLASSES)
    rank_u8 = np.round(rank / (CLASSES - 1) * 255).astype(np.uint8)

    e = np.concatenate([np.load(p)["birdnet"].reshape(-1, 1024) for p in embs]).astype(np.float32)
    e /= np.maximum(np.linalg.norm(e, axis=1, keepdims=True), 1e-9)
    proj = e @ axes.T
    ref = np.percentile(proj, QUANTILES, axis=0).T.astype(np.float32)  # N x Q

    with open(out_path, "wb") as f:
        f.write(b"OPC1")
        f.write(struct.pack("<4H", N, 1024, CLASSES, len(QUANTILES)))
        f.write(pca.explained_variance_ratio_.astype("<f4").tobytes())
        f.write(axes.astype("<f4").tobytes())
        f.write(ref.astype("<f4").tobytes())
        f.write(rank_u8.T.tobytes())
    print(f"wrote {out_path}, {os.path.getsize(out_path)} bytes, from {len(e)} reference readings")

    for k in range(N):
        lo = ", ".join(names[i] for i in order[:6, k])
        hi = ", ".join(names[i] for i in order[-6:, k][::-1])
        print(f"PC{k + 1} ({pca.explained_variance_ratio_[k] * 100:.1f}%)\n  low:  {lo}\n  high: {hi}")

    # Check: for confident readings, the reading's percentile (by the reference
    # quantiles) against its top class's percentile, and against a shuffle.
    raw = np.concatenate([np.load(p)["birdnet"].reshape(-1, 1024) for p in embs]).astype(np.float32)
    logit = raw @ w + bias
    top = logit.argmax(1)
    keep = 1 / (1 + np.exp(-logit.max(1))) >= 0.5
    rng = np.random.default_rng(1)
    print(f"check on {keep.sum()} readings with a class at 0.5 or more:")
    for k in range(N):
        p = np.interp(proj[keep, k], ref[k], QUANTILES / 100)
        r = rank[top[keep], k] / (CLASSES - 1)
        d = np.abs(p - r).mean()
        ds = np.abs(p - rng.permutation(r)).mean()
        c = np.corrcoef(p, r)[0, 1]
        print(f"  PC{k + 1}: mean gap {d:.3f}, shuffled {ds:.3f}, correlation {c:.2f}")


if __name__ == "__main__":
    main()
