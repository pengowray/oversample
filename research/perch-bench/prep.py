"""Turn audio files into model input windows (raw little-endian float32).

Perch v2 takes 160000 samples (5 s at 32 kHz); BirdNET V2.4 takes 144000 (3 s at 48 kHz).

  python prep.py out.f32 a.wav b.wav ...                     # Perch: 32 kHz, 160000
  python prep.py --rate 48000 --win 144000 out.f32 a.wav     # BirdNET
  python prep.py --rate 320000 out.f32 bat.wav               # bats: resample to 320 kHz,
                                                              # then read as 32 kHz (10x slower)
  python prep.py --repeat 36 out.f32 a.wav                   # repeat windows (speed tests)
"""

import argparse
from math import gcd

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("inputs", nargs="+")
ap.add_argument("--rate", type=int, default=32000, help="resample to this rate before windowing")
ap.add_argument("--win", type=int, default=160000, help="samples per window")
ap.add_argument("--repeat", type=int, default=0, help="repeat the windows until there are this many")
ap.add_argument("--max-windows", type=int, default=0, help="keep at most this many windows per file")
ap.add_argument("--list", default=None, help="write each window's source file, one per line")
a = ap.parse_args()
WIN = a.win

windows = []
for path in a.inputs:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != a.rate:
        g = gcd(sr, a.rate)
        x = resample_poly(x, a.rate // g, sr // g).astype(np.float32)
    n = int(np.ceil(len(x) / WIN))
    x = np.pad(x, (0, n * WIN - len(x)))
    if a.max_windows:
        n = min(n, a.max_windows)
    windows.append(x[: n * WIN].reshape(n, WIN))
    print(f"{path}: {sr} Hz -> {a.rate} Hz, {n} windows")

if a.list:
    with open(a.list, "w", encoding="utf-8") as f:
        for path, ws in zip(a.inputs, windows):
            f.write(f"{path}\n" * len(ws))

w = np.concatenate(windows)
if a.repeat:
    w = np.resize(w, (a.repeat, WIN))
w.astype("<f4").tofile(a.out)
print(f"{a.out}: {len(w)} windows of {WIN} samples")
