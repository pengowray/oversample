"""Official reference: Google's perch_v2_cpu SavedModel in TensorFlow 2.20.

  pip install "tensorflow-cpu~=2.20.0" kagglehub
  python ref_tf.py windows.f32 out_prefix [--threads 1]

Downloads google/bird-vocalization-classifier/tensorFlow2/perch_v2_cpu with kagglehub
(no Kaggle account needed), runs every 160000-sample window, and writes
<out_prefix>.emb.f32 and <out_prefix>.label.f32 for compare.py.
"""

import argparse
import time

import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument("windows")
ap.add_argument("out")
ap.add_argument("--threads", type=int, default=1)
a = ap.parse_args()

import kagglehub  # noqa: E402
import tensorflow as tf  # noqa: E402

tf.config.threading.set_intra_op_parallelism_threads(a.threads)
tf.config.threading.set_inter_op_parallelism_threads(1)

path = kagglehub.model_download("google/bird-vocalization-classifier/tensorFlow2/perch_v2_cpu")
t0 = time.perf_counter()
model = tf.saved_model.load(path)
f = model.signatures["serving_default"]
print(f"loaded {path} in {time.perf_counter() - t0:.1f} s; outputs {list(f.structured_outputs)}")

x = np.fromfile(a.windows, dtype="<f4").reshape(-1, 160000)
emb, lab, times = [], [], []
for w in x:
    t = time.perf_counter()
    o = f(inputs=tf.constant(w[None, :]))
    times.append(time.perf_counter() - t)
    emb.append(o["embedding"].numpy()[0])
    lab.append(o["label"].numpy()[0])
t = np.array(times[1:]) * 1000
print(f"TF {tf.__version__} threads={a.threads}: per window median {np.median(t):.1f} ms ({len(x)} windows)")
np.array(emb, "<f4").tofile(a.out + ".emb.f32")
np.array(lab, "<f4").tofile(a.out + ".label.f32")
