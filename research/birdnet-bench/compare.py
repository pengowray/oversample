"""Compare saved logits files (raw float32, N x 6522) against a reference.

  python compare.py ref.f32 other.f32 [other2.f32 ...]
"""

import sys

import numpy as np

C = 6522
ref = np.fromfile(sys.argv[1], dtype="<f4").reshape(-1, C)
sig = lambda v: 1 / (1 + np.exp(-v))
for path in sys.argv[2:]:
    r = np.fromfile(path, dtype="<f4").reshape(-1, C)
    print(
        f"{path}: top-1 agrees {np.mean(r.argmax(1) == ref.argmax(1)) * 100:.0f}% "
        f"({len(r)} windows), max |logit diff| {np.abs(r - ref).max():.4f}, "
        f"max |conf diff| {np.abs(sig(r) - sig(ref)).max():.4f}"
    )
