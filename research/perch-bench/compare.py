"""Compare saved Perch outputs against a reference run (from ref.py or the native/wasm runs).

  python compare.py REF_PREFIX OTHER_PREFIX [OTHER_PREFIX ...] [--indices regional_indices.txt]

Each prefix has <prefix>.emb.f32 (N x 1536) and optionally <prefix>.label.f32. A regional
slice's labels are compared with the reference's logits at the indices in --indices.
Perch logits are softmax-trained, so this reports raw logit differences, not sigmoids.
"""

import argparse
import os

import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument("ref")
ap.add_argument("others", nargs="+")
ap.add_argument("--indices", default=None)
a = ap.parse_args()

C = 14795
re = np.fromfile(a.ref + ".emb.f32", dtype="<f4").reshape(-1, 1536)
rl = np.fromfile(a.ref + ".label.f32", dtype="<f4").reshape(-1, C)
idx = np.loadtxt(a.indices, dtype=int) if a.indices else None


def cos(u, v):
    return (u * v).sum(1) / np.linalg.norm(u, axis=1) / np.linalg.norm(v, axis=1)


for p in a.others:
    e = np.fromfile(p + ".emb.f32", dtype="<f4").reshape(-1, 1536)
    msg = f"{os.path.basename(p)}: embedding max |diff| {np.abs(e - re).max():.5f}, min cosine {cos(e, re).min():.7f}"
    if os.path.exists(p + ".label.f32"):
        l = np.fromfile(p + ".label.f32", dtype="<f4")
        l = l.reshape(len(e), -1)
        r = rl if l.shape[1] == C else rl[:, idx]
        msg += f"; label top-1 same {np.sum(l.argmax(1) == r.argmax(1))}/{len(l)}, max |logit diff| {np.abs(l - r).max():.5f}"
    print(msg)
