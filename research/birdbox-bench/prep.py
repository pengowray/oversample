"""Turn test recordings into 32 kHz mono float32 WAV files, so that every pipeline in
this spike (BirdBox's own CLI, its all-in-one demo graph, and the Rust code here) reads
the same samples. Resampling is done once, here, the way BirdBox's CLI does it
(`librosa.resample`, its default soxr_hq), and is not part of the comparison.

  python -I prep.py OUT_DIR NAME=PATH [NAME=PATH ...] [--max-seconds 120]

Writes OUT_DIR/NAME.wav (float32) and adds a line per file to OUT_DIR/list.tsv (name,
seconds, source rate, channels, source path).
"""

import argparse
import os

import librosa
import numpy as np
import soundfile as sf

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("items", nargs="+")
ap.add_argument("--max-seconds", type=float, default=120.0)
a = ap.parse_args()
os.makedirs(a.out, exist_ok=True)
rows = []
for item in a.items:
    name, path = item.split("=", 1)
    try:
        audio, sr = sf.read(path, dtype="float32", always_2d=True)
        channels = audio.shape[1]
        audio = audio.mean(axis=1)
    except Exception:
        audio, sr = librosa.load(path, sr=None, mono=False)
        channels = 1 if audio.ndim == 1 else audio.shape[0]
        audio = audio if audio.ndim == 1 else audio.mean(axis=0)
    audio = audio[: int(a.max_seconds * sr)].astype(np.float32)
    if sr != 32000:
        audio = librosa.resample(audio, orig_sr=sr, target_sr=32000).astype(np.float32)
    sf.write(os.path.join(a.out, name + ".wav"), audio, 32000, subtype="FLOAT")
    rows.append(f"{name}\t{len(audio) / 32000:.3f}\t{sr}\t{channels}\t{path}")
    print(rows[-1])
with open(os.path.join(a.out, "list.tsv"), "a") as f:
    f.write("\n".join(rows) + "\n")
