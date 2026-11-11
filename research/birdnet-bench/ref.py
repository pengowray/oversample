"""Reference runs of BirdNET V2.4 with TFLite (LiteRT) and Python onnxruntime.

  python ref.py MODELS_DIR windows.f32 [--threads 1] [--save logits_prefix]

MODELS_DIR holds BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite, the tphakala ONNX files
(BirdNET_v2.4_fp32_foldconv.onnx, BirdNET_v2.4_int8_arm_foldconv.onnx) and labels.txt.
Prints ms per 3 s window, top-1 labels per window, and agreement with TFLite.
"""

import argparse
import os
import resource
import time

import numpy as np

WIN = 144000

ap = argparse.ArgumentParser()
ap.add_argument("models")
ap.add_argument("windows")
ap.add_argument("--threads", type=int, default=1)
ap.add_argument("--save", default=None, help="write <prefix>.<runtime>.f32 logits")
ap.add_argument("--only", default=None, help="comma list: tflite,ort_fp32,ort_int8")
a = ap.parse_args()

x = np.fromfile(a.windows, dtype="<f4").reshape(-1, WIN)
labels = [l.strip() for l in open(os.path.join(a.models, "labels.txt"), encoding="utf-8")]


def rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024


def run_tflite():
    from ai_edge_litert.interpreter import Interpreter

    t0 = time.perf_counter()
    it = Interpreter(
        model_path=os.path.join(a.models, "BirdNET_GLOBAL_6K_V2.4_Model_FP32.tflite"),
        num_threads=a.threads,
    )
    it.allocate_tensors()
    inp = it.get_input_details()[0]["index"]
    out = it.get_output_details()[0]["index"]
    load = time.perf_counter() - t0
    res = []
    times = []
    for w in x:
        t = time.perf_counter()
        it.set_tensor(inp, w[None, :])
        it.invoke()
        res.append(it.get_tensor(out)[0].copy())
        times.append(time.perf_counter() - t)
    return load, times, np.array(res)


def run_ort(name):
    import onnxruntime as ort

    so = ort.SessionOptions()
    so.intra_op_num_threads = a.threads
    so.inter_op_num_threads = 1
    t0 = time.perf_counter()
    s = ort.InferenceSession(os.path.join(a.models, name), so, providers=["CPUExecutionProvider"])
    load = time.perf_counter() - t0
    res = []
    times = []
    for w in x:
        t = time.perf_counter()
        res.append(s.run(None, {"input": w[None, :]})[0][0])
        times.append(time.perf_counter() - t)
    return load, times, np.array(res)


runs = {
    "tflite": run_tflite,
    "ort_fp32": lambda: run_ort("BirdNET_v2.4_fp32_foldconv.onnx"),
    "ort_int8": lambda: run_ort("BirdNET_v2.4_int8_arm_foldconv.onnx"),
}
only = a.only.split(",") if a.only else list(runs)

results = {}
for name in only:
    load, times, logits = runs[name]()
    results[name] = logits
    t = np.array(times[1:] or times) * 1000  # drop the first (warm-up) window
    print(
        f"{name:9s} threads={a.threads} load {load * 1000:6.0f} ms  "
        f"per window: median {np.median(t):6.1f} ms, min {t.min():6.1f} ms  "
        f"({len(x)} windows, {len(x) * 3 / (np.sum(times)):.0f}x real time)  peak RSS so far {rss_mb():.0f} MB"
    )
    if a.save:
        logits.astype("<f4").tofile(f"{a.save}.{name}.f32")

ref_name = "tflite" if "tflite" in results else only[0]
ref = results[ref_name]
for name, r in results.items():
    if name == ref_name:
        continue
    top_same = np.mean(r.argmax(1) == ref.argmax(1)) * 100
    p, q = 1 / (1 + np.exp(-r)), 1 / (1 + np.exp(-ref))
    print(f"{name} vs {ref_name}: top-1 agrees {top_same:.0f}%, max |logit diff| {np.abs(r - ref).max():.4f}, max |conf diff| {np.abs(p - q).max():.4f}")

print("\ntop-1 per window (" + ref_name + "):")
for i, row in enumerate(ref[: min(len(ref), 20)]):
    k = row.argmax()
    print(f"  {i * 3:4d}s  {1 / (1 + np.exp(-row[k])):.2f}  {labels[k]}")
