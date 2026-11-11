"""Write Perch's classifier (from split.py's perch_v2_head.npz) as one int8 file that
src/head.rs reads.

Layout, little-endian:
  b"PERCHHD1", u32 dim (1536), u32 classes (14795), u32 prototypes per class (4)
  f32 scale[classes * 4]    one per prototype: max |P column| / 127
  f32 W[classes * 4]
  f32 b[classes]
  i8  q[classes * 4][dim]   P transposed, so each prototype's 1536 values are together

  python export_head.py MODELS_DIR    # writes MODELS_DIR/perch_v2_head.int8.bin
"""

import os
import sys

import numpy as np

models = sys.argv[1]
H = np.load(os.path.join(models, "perch_v2_head.npz"))
P, W, b = H["P"], H["W"], H["b"]
dim, cols = P.shape
classes, protos = W.shape
assert cols == classes * protos

scale = np.abs(P).max(0) / 127
scale[scale == 0] = 1
q = np.round(P / scale).astype(np.int8).T.copy()

path = os.path.join(models, "perch_v2_head.int8.bin")
with open(path, "wb") as f:
    f.write(b"PERCHHD1")
    f.write(np.array([dim, classes, protos], dtype="<u4").tobytes())
    for a in (scale, W.reshape(-1), b):
        f.write(a.astype("<f4").tobytes())
    f.write(q.tobytes())
print(f"{path}: {os.path.getsize(path) / 1e6:.1f} MB")
