"""Reference runs of Perch v2: TFLite (LiteRT), Python onnxruntime, and the split
embedding model + numpy head (see split.py).

  python ref.py MODELS_DIR windows.f32 [--threads 1] [--save prefix] [--only tflite,ort_full,...]

MODELS_DIR holds perch_v2.tflite, perch_v2.onnx, perch_v2_no_dft.onnx (justinchuby
conversions), perch_v2_embed.onnx and perch_v2_head.npz (from split.py), and labels.csv.

Saves <prefix>.<run>.label.f32 (N x 14795 logits) and <prefix>.<run>.emb.f32 (N x 1536).
Perch logits come from a softmax-trained head, so results compare raw logits, top-1, and
embedding cosine similarity, not sigmoid confidences.
"""

import argparse
import os
import resource
import time

import numpy as np

WIN = 160000
C = 14795

ap = argparse.ArgumentParser()
ap.add_argument("models")
ap.add_argument("windows")
ap.add_argument("--threads", type=int, default=1)
ap.add_argument("--save", default=None)
ap.add_argument("--only", default=None)
a = ap.parse_args()

x = np.fromfile(a.windows, dtype="<f4").reshape(-1, WIN)
labels = [l.strip() for l in open(os.path.join(a.models, "labels.csv"), encoding="utf-8")][1:]


def rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024


def head_numpy(spatial, H):
    s = spatial.reshape(spatial.shape[0], -1, 1536)
    s = s / (np.sqrt((s * s).sum(-1, keepdims=True)) + 1e-5)
    p = (s @ H["P"]).reshape(s.shape[0], s.shape[1], C, 4).max(axis=1)
    return (p * H["W"][None]).sum(-1) + H["b"]


def run_tflite():
    from ai_edge_litert.interpreter import Interpreter

    t0 = time.perf_counter()
    it = Interpreter(model_path=os.path.join(a.models, "perch_v2.tflite"), num_threads=a.threads)
    it.allocate_tensors()
    inp = it.get_input_details()[0]["index"]
    outs = {tuple(d["shape"][1:]): d["index"] for d in it.get_output_details()}
    load = time.perf_counter() - t0
    lab, emb, times = [], [], []
    for w in x:
        t = time.perf_counter()
        it.set_tensor(inp, w[None, :])
        it.invoke()
        lab.append(it.get_tensor(outs[(C,)])[0].copy())
        emb.append(it.get_tensor(outs[(1536,)])[0].copy())
        times.append(time.perf_counter() - t)
    return load, times, np.array(lab), np.array(emb)


def session(name):
    import onnxruntime as ort

    so = ort.SessionOptions()
    so.intra_op_num_threads = a.threads
    so.inter_op_num_threads = 1
    return ort.InferenceSession(os.path.join(a.models, name), so, providers=["CPUExecutionProvider"])


def run_ort(name):
    t0 = time.perf_counter()
    s = session(name)
    load = time.perf_counter() - t0
    lab, emb, times = [], [], []
    for w in x:
        t = time.perf_counter()
        e, sp, spec, l = s.run(["embedding", "spatial_embedding", "spectrogram", "label"], {"inputs": w[None, :]})
        times.append(time.perf_counter() - t)
        lab.append(l[0])
        emb.append(e[0])
    return load, times, np.array(lab), np.array(emb)


def run_split(time_head):
    t0 = time.perf_counter()
    s = session("perch_v2_embed.onnx")
    H = dict(np.load(os.path.join(a.models, "perch_v2_head.npz")))
    load = time.perf_counter() - t0
    lab, emb, times = [], [], []
    for w in x:
        t = time.perf_counter()
        e, sp = s.run(["embedding", "spatial_embedding"], {"inputs": w[None, :]})
        if not time_head:
            times.append(time.perf_counter() - t)
        l = head_numpy(sp, H)
        if time_head:
            times.append(time.perf_counter() - t)
        lab.append(l[0])
        emb.append(e[0])
    return load, times, np.array(lab), np.array(emb)


runs = {
    "tflite": run_tflite,
    "ort_full": lambda: run_ort("perch_v2.onnx"),
    "ort_nodft": lambda: run_ort("perch_v2_no_dft.onnx"),
    "ort_embed": lambda: run_split(False),
    "ort_embed_numpyhead": lambda: run_split(True),
}
only = a.only.split(",") if a.only else list(runs)

res = {}
for name in only:
    load, times, lab, emb = runs[name]()
    res[name] = (lab, emb)
    t = np.array(times[1:] or times) * 1000
    print(
        f"{name:20s} threads={a.threads} load {load * 1000:6.0f} ms  per window: median {np.median(t):6.1f} ms, "
        f"min {t.min():6.1f} ms  ({len(x)} windows, {len(x) * 5 / np.sum(times):.0f}x real time)  peak RSS so far {rss_mb():.0f} MB"
    )
    if a.save:
        lab.astype("<f4").tofile(f"{a.save}.{name}.label.f32")
        emb.astype("<f4").tofile(f"{a.save}.{name}.emb.f32")


def cos(u, v):
    return (u * v).sum(1) / np.linalg.norm(u, axis=1) / np.linalg.norm(v, axis=1)


ref = only[0]
rl, re = res[ref]
for name, (l, e) in res.items():
    if name == ref:
        continue
    print(
        f"{name} vs {ref}: top-1 agrees {np.sum(l.argmax(1) == rl.argmax(1))}/{len(l)}, "
        f"max |logit diff| {np.abs(l - rl).max():.5f}, min embedding cosine {cos(e, re).min():.6f}"
    )

print(f"\ntop-1 per window ({ref}), logit and softmax probability:")
for i, row in enumerate(rl):
    k = row.argmax()
    p = np.exp(row - row.max())
    print(f"  {i:3d}  {row[k]:6.2f}  {p[k] / p.sum():.2f}  {labels[k]}")
