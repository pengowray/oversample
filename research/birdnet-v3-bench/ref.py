"""Reference scores for BirdNET+ V3.0 ONNX files with onnxruntime.

Runs each window (3 s, 96,000 samples) through the model, one at a time, on the given
number of threads, and saves the 1,280-value embeddings and the class scores:

  <prefix>.emb.f32    N x 1280
  <prefix>.score.f32  N x classes

  python -I ref.py MODEL.onnx windows.f32 --save out/birds.fp32 [--threads 1]
         [--labels Labels.csv] [--list birds.list] [--top 5]

With --labels and --list, prints the top classes for each window.
"""

import argparse
import csv
import time

import numpy as np
import onnxruntime as ort

WIN = 96000

ap = argparse.ArgumentParser()
ap.add_argument("model")
ap.add_argument("windows")
ap.add_argument("--save", default=None)
ap.add_argument("--threads", type=int, default=1)
ap.add_argument("--labels", default=None)
ap.add_argument("--list", default=None)
ap.add_argument("--top", type=int, default=5)
a = ap.parse_args()

w = np.fromfile(a.windows, "<f4").reshape(-1, WIN)
so = ort.SessionOptions()
so.intra_op_num_threads = a.threads
so.inter_op_num_threads = 1
t0 = time.perf_counter()
s = ort.InferenceSession(a.model, so, providers=["CPUExecutionProvider"])
load = time.perf_counter() - t0
names = [o.name for o in s.get_outputs()]
inp = s.get_inputs()[0].name

emb, sc, times = [], [], []
for x in w:
    t = time.perf_counter()
    out = dict(zip(names, s.run(None, {inp: x[None, :]})))
    times.append(time.perf_counter() - t)
    emb.append(out["embeddings"][0].astype(np.float32))
    sc.append(out["predictions"][0].astype(np.float32))
emb, sc = np.stack(emb), np.stack(sc)
med = np.median(times[1:] if len(times) > 1 else times) * 1000
print(
    f"{a.model.split('/')[-1]}: onnxruntime {ort.__version__}, {a.threads} thread(s), "
    f"load {load:.2f} s, median {med:.1f} ms per 3 s window, {sc.shape[1]} classes, "
    f"embedding {emb.shape[1]}, any NaN: {bool(np.isnan(sc).any() or np.isnan(emb).any())}"
)
if a.save:
    emb.tofile(a.save + ".emb.f32")
    sc.tofile(a.save + ".score.f32")

if a.labels:
    with open(a.labels, encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    lab = [r.get("common_name") or r["scientific_name"] for r in rows]
    src = [""] * len(w)
    if a.list:
        src = [ln.rstrip("\n").split("/")[-1] for ln in open(a.list, encoding="utf-8")]
    for i, row in enumerate(sc):
        top = np.argsort(-row)[: a.top]
        print(f"{i:2d} {src[i][:40]:40s} " + "; ".join(f"{lab[j]} {row[j]:.3f}" for j in top))
