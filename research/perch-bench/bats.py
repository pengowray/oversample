"""Do Perch v2 embeddings separate bat species better than BirdNET V2.4's?

Bat recordings are time-expanded before embedding: resampled to RATE and then read
as the model's own rate (32 kHz for Perch, 48 kHz for BirdNET). For example Perch at
256 kHz is an 8x expansion; 160000 samples then hold 0.625 s of real time and Perch's
60 Hz to 16 kHz mel range covers 0.5 to 128 kHz of the real recording.

Recordings: every xeno-canto bat file in SOUNDS_DIR that is WAV or FLAC, recorded at
192 kHz or more, of a species that has at least 2 such files. Per recording, the 3
windows with the most energy between 15 and 120 kHz are embedded, and their mean is the
recording's embedding (most windows of a bat recording hold no call).

Scores (cosine distance, after subtracting the mean embedding of all recordings):
- species 1-NN: the nearest other recording is the same species (leave one out)
- genus 1-NN: ... the same genus
- the same, split by whether a species' recordings share a recordist
- silhouette: how tight each species is compared with the nearest other species

  python bats.py MODELS_DIR SOUNDS_DIR out.npz
"""

import collections
import glob
import json
import os
import sys
import time
from math import gcd

import numpy as np
import onnxruntime as ort
import soundfile as sf
from scipy.signal import resample_poly
from sklearn.metrics import silhouette_score

models, sounds, out = sys.argv[1], sys.argv[2], sys.argv[3]
TOP = 3
BAND = (15000, 120000)

# --- recordings ---
recs = []
for p in sorted(glob.glob(os.path.join(sounds, "*.xc.json"))):
    d = json.load(open(p, encoding="utf-8"))
    base = p[: -len(".xc.json")]
    audio = [base + e for e in (".wav", ".flac") if os.path.exists(base + e)]
    sp = f"{d.get('gen', '')} {d.get('sp', '')}".strip()
    if d.get("grp") != "bats" or not audio or sp in ("Mystery mystery", "Sonus naturalis"):
        continue
    if int(d.get("smp") or 0) < 192000:
        continue
    recs.append(dict(path=audio[0], species=sp, genus=d.get("gen"), rec=d.get("rec"), xc=d.get("xc_id"), sr=int(d["smp"])))
count = collections.Counter(r["species"] for r in recs)
recs = [r for r in recs if count[r["species"]] >= 2]
print(f"{len(recs)} recordings, {len(set(r['species'] for r in recs))} species")


def load(path, rate):
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != rate:
        g = gcd(sr, rate)
        x = resample_poly(x, rate // g, sr // g).astype(np.float32)
    return x, sr


def top_windows(x, rate, win, sr):
    """The TOP windows (50% overlap) with the most energy in BAND (capped at the file's Nyquist)."""
    if len(x) < win:
        x = np.pad(x, (0, win - len(x)))
    hop = win // 2
    starts = list(range(0, len(x) - win + 1, hop)) or [0]
    lo, hi = BAND[0], min(BAND[1], sr / 2 * 0.98)
    f = np.fft.rfftfreq(win, 1 / rate)
    band = (f >= lo) & (f <= hi)
    e = [np.sum(np.abs(np.fft.rfft(x[s : s + win]))[band] ** 2) for s in starts]
    best = np.argsort(e)[::-1][:TOP]
    return np.stack([x[starts[i] : starts[i] + win] for i in sorted(best)])


so = ort.SessionOptions()
so.intra_op_num_threads = 4
perch = ort.InferenceSession(os.path.join(models, "perch_v2_embed.onnx"), so)
birdnet = ort.InferenceSession(os.path.join(models, "birdnet-v2.4-embeddings-fp32.onnx"), so)


def emb_perch(w):
    return perch.run(["embedding"], {"inputs": w})[0]


def emb_birdnet(w):
    return birdnet.run(["model/GLOBAL_AVG_POOL/Mean_reduced_0"], {"input": w})[0]


def spectrum(w, rate):
    """Baseline: mean log power spectrum, 1 kHz bands from 10 to 128 kHz (real time)."""
    f = np.fft.rfftfreq(w.shape[1], 1 / rate)
    p = np.abs(np.fft.rfft(w, axis=1)) ** 2
    edges = np.arange(10000, 128001, 1000)
    bands = np.stack([p[:, (f >= a) & (f < b)].mean(1) for a, b in zip(edges[:-1], edges[1:])], 1)
    return np.log(bands + 1e-12)


configs = {
    # name: (embed fn, resample rate, window samples, model rate)
    "Perch, 8x (256 kHz read as 32 kHz)": (emb_perch, 256000, 160000, 32000),
    "Perch, 10x (320 kHz read as 32 kHz)": (emb_perch, 320000, 160000, 32000),
    "BirdNET, 5.33x (256 kHz read as 48 kHz, as BattyBirdNET)": (emb_birdnet, 256000, 144000, 48000),
    "BirdNET, 10x (480 kHz read as 48 kHz)": (emb_birdnet, 480000, 144000, 48000),
    "Baseline: mean log spectrum": (None, 256000, 160000, None),
}

E = {}
cache = {}
for name, (fn, rate, win, _) in configs.items():
    t = time.perf_counter()
    vs = []
    for r in recs:
        key = (r["path"], rate)
        if key not in cache:
            cache.clear()
            cache[key] = load(r["path"], rate)
        x, sr = cache[key]
        w = top_windows(x, rate, win, sr)
        v = spectrum(w, rate) if fn is None else fn(w)
        vs.append(v.mean(0))
    E[name] = np.array(vs, np.float32)
    print(f"{name}: {time.perf_counter() - t:.0f} s", flush=True)

sp = np.array([r["species"] for r in recs])
ge = np.array([r["genus"] for r in recs])
same_rec = {s: len({r["rec"] for r in recs if r["species"] == s}) == 1 for s in set(sp)}
one = np.array([same_rec[s] for s in sp])


def hits(X):
    X = X - X.mean(0)
    X = X / np.linalg.norm(X, axis=1, keepdims=True)
    S = X @ X.T
    np.fill_diagonal(S, -np.inf)
    nn = S.argmax(1)
    return X, nn, sp[nn] == sp


def scores(X):
    X, nn, hit = hits(X)
    D = np.clip(1 - X @ X.T, 0, None)
    np.fill_diagonal(D, 0)
    return dict(
        species=hit.mean(),
        genus=(ge[nn] == ge).mean(),
        same_recordist=hit[one].mean(),
        diff_recordist=hit[~one].mean(),
        silhouette=silhouette_score(D, sp, metric="precomputed"),
        genus_silhouette=silhouette_score(D, ge, metric="precomputed"),
    )


print(
    f"\n{len(recs)} recordings, {len(set(sp))} species, {len(set(ge))} genera; "
    f"{one.sum()} recordings from species whose files share one recordist, {(~one).sum()} from species with two recordists"
)
print(f"chance for species 1-NN: about {1 / (len(recs) - 1):.3f}\n")
print(f"{'':60s} species  genus  same-rec  diff-rec  silh(sp)  silh(genus)")
res = {}
for name, X in E.items():
    s = scores(X)
    res[name] = s
    print(
        f"{name:60s} {s['species']:.3f}   {s['genus']:.3f}  {s['same_recordist']:.3f}     {s['diff_recordist']:.3f}     "
        f"{s['silhouette']:+.3f}    {s['genus_silhouette']:+.3f}"
    )

# Paired: recordings one model gets right (species 1-NN) and the other gets wrong.
names = list(E)
print("\npaired species 1-NN, all recordings / recordings from species with two recordists:")
for i, a_ in enumerate(names):
    for b_ in names[i + 1 :]:
        ha, hb = hits(E[a_])[2], hits(E[b_])[2]
        print(
            f"  {a_.split(',')[0]} {a_.split(',')[1].split('(')[0].strip() if ',' in a_ else ''} vs "
            f"{b_.split(',')[0]} {b_.split(',')[1].split('(')[0].strip() if ',' in b_ else ''}: "
            f"only first right {np.sum(ha & ~hb)}, only second right {np.sum(~ha & hb)} / "
            f"{np.sum((ha & ~hb)[~one])}, {np.sum((~ha & hb)[~one])}"
        )
np.savez(out,species=sp, genus=ge, xc=np.array([r["xc"] for r in recs]), **{k: v for k, v in E.items()})
