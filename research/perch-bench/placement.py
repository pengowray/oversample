"""Where should a detection go on the Perch species map?

The map's points are each class's weighted-mean prototype (species_map.py). For each
window, compare the model's top-1 class with the class whose map vector is closest to:
  pooled: the 1536-value pooled `embedding`
  best:   the best single position of the 16 x 4 spatial embedding
Reported for all windows and for windows whose top-1 softmax probability is 0.3 or more.

  python placement.py MODELS_DIR windows.f32
"""

import os
import sys

import numpy as np
import onnxruntime as ort

models, wins = sys.argv[1], sys.argv[2]
x = np.fromfile(wins, dtype="<f4").reshape(-1, 160000)
H = np.load(os.path.join(models, "perch_v2_head.npz"))
P = H["P"].reshape(1536, -1, 4).transpose(1, 2, 0)
W = H["W"]
mean_v = (P * W[:, :, None]).sum(1)
n = np.linalg.norm(mean_v, axis=1, keepdims=True)
mean_v = mean_v / np.where(n == 0, 1, n)

s = ort.InferenceSession(os.path.join(models, "perch_v2.onnx"))
rows = []
for w in x:
    e, sp, lab = s.run(["embedding", "spatial_embedding", "label"], {"inputs": w[None]})
    p = np.exp(lab[0] - lab[0].max())
    p /= p.sum()
    top = lab[0].argmax()
    pooled = (mean_v @ (e[0] / np.linalg.norm(e[0]))).argmax()
    spn = sp[0].reshape(-1, 1536)
    spn = spn / np.linalg.norm(spn, axis=1, keepdims=True)
    best = (spn @ mean_v.T).max(0).argmax()
    rank = lambda c: int((np.argsort(-lab[0]) == c).nonzero()[0][0]) + 1
    rows.append((p[top], pooled == top, best == top, rank(pooled), rank(best)))

r = np.array(rows, dtype=float)
conf = r[:, 0] >= 0.3
for name, m in (("all windows", np.ones(len(r), bool)), ("top-1 probability >= 0.3", conf)):
    print(
        f"{name} ({m.sum()}): nearest map vector = top-1 class for pooled {int(r[m, 1].sum())}, "
        f"best position {int(r[m, 2].sum())}; median rank of that class in the logits: "
        f"pooled {np.median(r[m, 3]):.0f}, best position {np.median(r[m, 4]):.0f}"
    )
