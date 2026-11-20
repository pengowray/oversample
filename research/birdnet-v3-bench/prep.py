"""Turn audio files into BirdNET V3.0 input windows: 3 s of 32 kHz mono float32
(96,000 samples), raw little-endian, back to back. With --quiet, adds one
window of digital silence and windows of white noise at -90 and -60 dBFS RMS.

  python -I prep.py out.f32 a.wav b.mp3 ... [--max-windows 6] [--quiet] [--list out.list]
  python -I prep.py --repeat 60 long.f32 a.wav      # 3 minutes, for timing

Reads anything libsndfile reads (WAV, FLAC, Ogg, MP3). Channels are averaged.
"""

import argparse
from math import gcd

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

RATE = 32000
WIN = 96000

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("inputs", nargs="*")
ap.add_argument("--max-windows", type=int, default=0)
ap.add_argument("--repeat", type=int, default=0, help="repeat the windows until there are this many")
ap.add_argument("--quiet", action="store_true", help="add a silent window and low-noise windows")
ap.add_argument("--noise-dbfs", type=float, nargs="+", default=[-90.0, -60.0], help="RMS of each noise window")
ap.add_argument("--list", default=None, help="write each window's source, one per line")
a = ap.parse_args()

windows, names = [], []
for path in a.inputs:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != RATE:
        g = gcd(sr, RATE)
        x = resample_poly(x, RATE // g, sr // g).astype(np.float32)
    secs = len(x) / RATE
    n = int(np.ceil(len(x) / WIN))
    x = np.pad(x, (0, n * WIN - len(x)))
    if a.max_windows:
        n = min(n, a.max_windows)
    windows.append(x[: n * WIN].reshape(n, WIN))
    names += [path] * n
    print(f"{path}: {sr} Hz, {secs:.1f} s, {n} windows")

if a.quiet:
    windows.append(np.zeros((1, WIN), np.float32))
    names.append("silence (all zeros)")
    rng = np.random.default_rng(0)
    for db in a.noise_dbfs:
        rms = 10 ** (db / 20)
        windows.append((rng.standard_normal((1, WIN)) * rms).astype(np.float32))
        names.append(f"white noise, {db:g} dBFS RMS")

w = np.concatenate(windows)
if a.repeat:
    w = np.resize(w, (a.repeat, WIN))
    names = [names[i % len(names)] for i in range(a.repeat)]
w.astype("<f4").tofile(a.out)
if a.list:
    with open(a.list, "w", encoding="utf-8") as f:
        f.write("".join(n + "\n" for n in names))
print(f"{a.out}: {len(w)} windows of {WIN} samples")
