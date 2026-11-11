"""Does the Perch classifier still give the same answers stored in float16 or int8?

The classifier's prototype matrix P (1536 x 59180, 364 MB in float32) is most of the
download. This runs the embedding model once per window, then the classifier with P
stored as float32, float16, and int8 (one scale per prototype column), and compares
the logits with float32.

  python head_precision.py MODELS_DIR windows.f32
"""

import os
import sys

import numpy as np
import onnxruntime as ort

models, wins = sys.argv[1], sys.argv[2]
x = np.fromfile(wins, dtype="<f4").reshape(-1, 160000)
H = np.load(os.path.join(models, "perch_v2_head.npz"))
P, W, b = H["P"], H["W"], H["b"]
C = W.shape[0]

s = ort.InferenceSession(os.path.join(models, "perch_v2_embed.onnx"))
sp = np.concatenate([s.run(["spatial_embedding"], {"inputs": w[None]})[0] for w in x])
sp = sp.reshape(len(x), -1, 1536)
sp = sp / (np.sqrt((sp * sp).sum(-1, keepdims=True)) + 1e-5)


def head(Pm):
    out = []
    for v in sp:
        p = (v @ Pm).reshape(v.shape[0], C, 4).max(0)
        out.append((p * W).sum(-1) + b)
    return np.array(out)


ref = head(P)
p16 = P.astype(np.float16)
scale = np.abs(P).max(0) / 127
q8 = np.round(P / scale).astype(np.int8)
variants = {
    "float16": (p16.astype(np.float32), p16.nbytes),
    "int8, one scale per prototype": (q8.astype(np.float32) * scale, q8.nbytes + scale.nbytes),
}
top5 = lambda l: np.argsort(-l, axis=1)[:, :5]
print(f"float32: {P.nbytes / 1e6:.0f} MB")
for name, (Pm, nbytes) in variants.items():
    l = head(Pm)
    same5 = np.mean([set(a) == set(c) for a, c in zip(top5(l), top5(ref))])
    print(
        f"{name}: {nbytes / 1e6:.0f} MB, top-1 same {np.sum(l.argmax(1) == ref.argmax(1))}/{len(l)}, "
        f"same top-5 set {same5 * len(l):.0f}/{len(l)}, max |logit diff| {np.abs(l - ref).max():.4f}"
    )
