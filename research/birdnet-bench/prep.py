"""Turn audio files into BirdNET input windows.

Writes one raw little-endian float32 file holding N windows of 144000 samples
(3 s at 48 kHz), the input shape of BirdNET V2.4.

  python prep.py out.f32 a.wav b.wav ...           # resample to 48 kHz
  python prep.py --rate 256000 out.f32 bat.wav     # BattyBirdNET: resample to 256 kHz,
                                                    # then feed as if it were 48 kHz
  python prep.py --repeat 60 out.f32 a.wav         # repeat windows (speed tests)
"""

import argparse
from math import gcd

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

WIN = 144000

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("inputs", nargs="+")
ap.add_argument("--rate", type=int, default=48000, help="resample to this rate before windowing")
ap.add_argument("--repeat", type=int, default=0, help="repeat the windows until there are this many")
a = ap.parse_args()

windows = []
for path in a.inputs:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != a.rate:
        g = gcd(sr, a.rate)
        x = resample_poly(x, a.rate // g, sr // g).astype(np.float32)
    n = int(np.ceil(len(x) / WIN))
    x = np.pad(x, (0, n * WIN - len(x)))
    windows.append(x.reshape(n, WIN))
    print(f"{path}: {sr} Hz -> {a.rate} Hz, {n} windows")

w = np.concatenate(windows)
if a.repeat:
    w = np.resize(w, (a.repeat, WIN))
w.astype("<f4").tofile(a.out)
print(f"{a.out}: {len(w)} windows ({len(w) * 3} s at 48 kHz)")
