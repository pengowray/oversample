"""Rough check of BattyBirdNET (BirdNET V2.4 embeddings + a regional bat head).

Each recording is resampled to 256 kHz and cut into 144000-sample windows (0.5625 s),
which BirdNET reads as 3 s of 48 kHz audio: a 5.33x time expansion. Prints the top
species per file, scored as the highest window score for each class.

  python batty.py MODELS_DIR REGION a.wav b.wav ...      # REGION e.g. EU, UK
"""

import os
import sys
import time
from math import gcd

import numpy as np
import onnxruntime as ort
import soundfile as sf
from scipy.signal import resample_poly

WIN = 144000
RATE = 256000
models, region, files = sys.argv[1], sys.argv[2], sys.argv[3:]
emb = ort.InferenceSession(os.path.join(models, "birdnet-v2.4-embeddings-fp32.onnx"))
head = ort.InferenceSession(os.path.join(models, f"fp32/BattyBirdNET-{region}-256kHz_fp32.onnx"))
labels = [l.strip() for l in open(os.path.join(models, f"labels/BattyBirdNET-{region}-256kHz_Labels.txt"))]
out_names = [o.name for o in emb.get_outputs()]
emb_idx = 1 if len(out_names) > 1 else 0

hits = 0
for path in files:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != RATE:
        g = gcd(sr, RATE)
        x = resample_poly(x, RATE // g, sr // g).astype(np.float32)
    n = max(1, len(x) // WIN)
    w = np.pad(x, (0, max(0, n * WIN - len(x))))[: n * WIN].reshape(n, WIN)
    t = time.perf_counter()
    e = np.concatenate([emb.run(None, {"input": w[i : i + 1]})[emb_idx] for i in range(n)])
    s = 1 / (1 + np.exp(-head.run(None, {head.get_inputs()[0].name: e})[0]))
    ms = (time.perf_counter() - t) * 1000 / n
    best = s.max(axis=0)
    order = np.argsort(-best)[:3]
    name = os.path.basename(path).split(" - ")[-1].removesuffix(".wav")
    top = labels[order[0]].split("_")[0]
    hits += top == name
    print(
        f"{name:28s} {sr // 1000:3d} kHz {n:3d} win {ms:5.1f} ms/win  "
        + ", ".join(f"{labels[k].split('_')[0]} {best[k]:.2f}" for k in order)
        + ("  <- match" if top == name else "")
    )
print(f"top-1 matches the XC label in {hits} of {len(files)} files")
