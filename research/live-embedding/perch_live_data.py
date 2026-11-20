"""Data for the live view with Perch 2.0: a UMAP layout of Perch's classes
(perch-umap.bin) and the principal components of its class vectors
(perch-pc-rows.bin). The Perch versions of species_umap.py and pc_bands.py.

Class vector: the weighted mean of each class's 4 prototypes, scaled to
length 1, as research/species-map/project_perch.py makes for perch-map.bin.
The 53 classes whose prototype weights are all 0 are left out of the PCA and
the layout (written as "no position", ranked in the middle).

  python -I perch_live_data.py HEAD.npz LABELS.csv EBIRD_CLASSES.csv \\
      BIRDNET_LABELS.txt TSNE_MAP.bin OUT_DIR EMB.npz [EMB.npz ...] \\
      [--spatial SPATIAL.npy]

HEAD.npz is perch_v2_head.npz (research/perch-bench/split.py: P 1536 x
59,180, W 14,795 x 4, b). LABELS.csv and EBIRD_CLASSES.csv are cgeorgiaw/Perch
`assets/`. BIRDNET_LABELS.txt gives English names for the printout only.
TSNE_MAP.bin is research/species-map/perch-map.bin. EMB.npz files hold Perch's
pooled embeddings of real recordings under key "perch" (BirdSet HSN and POW
test sets, batchi-noisier2noise/birdset_embed.py, raw arm). SPATIAL.npy is the
matching spatial embedding [N, 64, 1536] of the first EMB.npz, for the check
(reading percentile against its top class's percentile).

UMAP runs in node with the app's vendored umap-js (umap_js.cjs), on the
class vectors after PCA to 50 dimensions and scaling back to length 1, so
euclidean distance there follows cosine distance. umap-learn was not
installed on the machine this was made on.

Writes, in OUT_DIR:
  perch-umap.bin      14,795 x 2 int16 (as perch-map.bin), the layout
  perch-pc-rows.bin   OPC1 file (pc_bands.py's layout), D = 1536, C = 14,795

Needs numpy, scikit-learn, and node.
"""

import os
import re
import struct
import subprocess
import sys
import tempfile
import time

import numpy as np
from sklearn.decomposition import PCA
from sklearn.neighbors import NearestNeighbors

HERE = os.path.dirname(os.path.abspath(__file__))
SEED = 1
K = 10
MAX_N = 16
QUANTILES = np.linspace(0, 100, 21)
# A component gets a map while the share of a species' 10 nearest
# neighbours in the same half stays at or above this on the layout (BirdNET's
# file stopped at PC12, the last at 0.65; past it 0.58 to 0.63).
KEEP_COMPACT = 0.65
SETTINGS = [(15, 0.1), (50, 0.1)]


def unit(x):
    n = np.linalg.norm(x, axis=-1, keepdims=True)
    return x / np.where(n == 0, 1, n)


def read_map(path):
    raw = np.frombuffer(open(path, "rb").read(), "<i2").reshape(-1, 2)
    xy = raw.astype(np.float32) / 32767
    xy[(raw == -32768).all(1)] = np.nan
    return xy


def compactness(xy, p, bands=2):
    nn = NearestNeighbors(n_neighbors=K + 1).fit(xy)
    _, idx = nn.kneighbors(xy)
    idx = idx[:, 1:]
    out = []
    for k in range(p.shape[1]):
        band = np.minimum(np.floor(p[:, k] * bands), bands - 1)
        out.append(float((band[idx] == band[:, None]).mean()))
    return out


def umap_js(x, nn, md):
    with tempfile.TemporaryDirectory() as d:
        src, dst = os.path.join(d, "in.f32"), os.path.join(d, "out.f32")
        x.astype("<f4").tofile(src)
        cmd = ["node", os.path.join(HERE, "umap_js.cjs"), src, str(len(x)), str(x.shape[1]), dst, str(nn), str(md), str(SEED)]
        print(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip(), flush=True)
        return np.fromfile(dst, "<f4").reshape(-1, 2)


def nth_neighbour(xy, n):
    d, _ = NearestNeighbors(n_neighbors=n + 1).fit(xy).kneighbors(xy)
    return float(np.median(d[:, n]))


def main():
    args = sys.argv[1:]
    spatial = None
    if "--spatial" in args:
        i = args.index("--spatial")
        spatial = args[i + 1]
        del args[i : i + 2]
    head, labels_csv, ebird_csv, birdnet_labels, tsne_path, out_dir, *embs = args
    h = np.load(head)
    P, W, bias = h["P"], h["W"], h["b"]
    C = W.shape[0]
    D = P.shape[0]
    Pc = P.reshape(D, C, 4).transpose(1, 2, 0)  # class, prototype, dim
    labels = [l.strip() for l in open(labels_csv, encoding="utf-8")][1:]
    codes = [l.strip() for l in open(ebird_csv, encoding="utf-8")][1:]
    assert len(labels) == C == len(codes)
    common = {}
    for line in open(birdnet_labels, encoding="utf-8"):
        sci, _, com = line.strip().partition("_")
        common[sci] = com
    binomial = re.compile(r"^[A-Z][a-z]+ [a-z-]+")

    def name(i):
        l = labels[i]
        if not binomial.match(l):
            return f"[sound] {l}"
        kind = "" if codes[i] != "no_ebird_code" else "[no eBird code] "
        c = common.get(l)
        return f"{kind}{l}" + (f" ({c})" if c else "")

    live = W.sum(1) > 0
    idx = np.nonzero(live)[0]
    V = unit((Pc * W[:, :, None]).sum(1))[idx]
    print(f"{C} classes, {len(idx)} live", flush=True)

    # Principal components of the live class vectors.
    pca = PCA(n_components=MAX_N, random_state=SEED).fit(V)
    axes = pca.components_.astype(np.float32)
    score = V @ axes.T
    order = np.argsort(score, axis=0)
    rank = np.empty_like(order)
    for k in range(MAX_N):
        rank[order[:, k], k] = np.arange(len(idx))
    pct = rank / (len(idx) - 1)
    shuffle_rng = np.random.default_rng(SEED)
    shuffled = PCA(n_components=4, random_state=SEED).fit(
        np.column_stack([shuffle_rng.permutation(V[:, j]) for j in range(D)])
    )
    print("variance, PC1..PC%d (%%): %s" % (MAX_N, " ".join(f"{v * 100:.2f}" for v in pca.explained_variance_ratio_)))
    print("same PCA with each column shuffled (%%): %s" % " ".join(f"{v * 100:.2f}" for v in shuffled.explained_variance_ratio_))

    # Layouts: the t-SNE map and UMAP on PCA-50.
    tsne = read_map(tsne_path)[idx]
    layouts = {"t-SNE": tsne}
    V50 = unit(PCA(n_components=50, random_state=SEED).fit_transform(V))
    for nn, md in SETTINGS:
        xy = umap_js(V50, nn, md)
        xy = xy - np.median(xy, axis=0)
        xy = np.clip(xy / np.percentile(np.abs(xy), 99.5), -1, 1)
        layouts[f"UMAP {nn}/{md}"] = xy.astype(np.float32)

    print(f"\nclass vectors, halves: share of {K} nearest neighbours in the same half, PC1..PC{MAX_N}")
    comp = {}
    for lname, xy in layouts.items():
        c = compactness(xy, pct)
        comp[lname] = c
        print(f"  {lname:14s} mean {np.mean(c[:12]):.2f} (PC1-12):  " + " ".join(f"{v:.2f}" for v in c))
    best = max((l for l in layouts if l != "t-SNE"), key=lambda l: np.mean(comp[l][:12]))
    print(f"chosen UMAP: {best}")
    xy = layouts[best]
    lo, hi = xy.min(0), xy.max(0)
    xy = (xy - (lo + hi) / 2) / ((hi - lo).max() / 2)
    full = np.full((C, 2), -32768, "<i2")
    full[idx] = np.round(xy * 32767).astype("<i2")
    umap_path = os.path.join(out_dir, "perch-umap.bin")
    full.tofile(umap_path)
    print(f"wrote {umap_path}")
    d_tsne, d_umap = nth_neighbour(tsne, K), nth_neighbour(xy, K)
    print(f"median distance to the {K}th nearest class: t-SNE {d_tsne:.4f}, UMAP {d_umap:.4f}, ratio {d_umap / d_tsne:.2f}")

    # Keep the leading components that still form areas on the UMAP layout.
    n = 0
    while n < MAX_N and comp[best][n] >= KEEP_COMPACT:
        n += 1
    n = max(n, 4)
    print(f"components kept: {n} (share of neighbours in the same half >= {KEEP_COMPACT} on {best})")

    # Reference recordings' embeddings, projected.
    e = np.concatenate([np.load(p)["perch"].reshape(-1, D) for p in embs]).astype(np.float32)
    e_unit = unit(e)
    proj = e_unit @ axes[:n].T
    ref = np.percentile(proj, QUANTILES, axis=0).T.astype(np.float32)
    rank_full = np.full((C, n), 128, np.uint8)
    rank_full[idx] = np.round(pct[:, :n] * 255).astype(np.uint8)
    pc_path = os.path.join(out_dir, "perch-pc-rows.bin")
    with open(pc_path, "wb") as f:
        f.write(b"OPC1")
        f.write(struct.pack("<4H", n, D, C, len(QUANTILES)))
        f.write(pca.explained_variance_ratio_[:n].astype("<f4").tobytes())
        f.write(axes[:n].astype("<f4").tobytes())
        f.write(ref.astype("<f4").tobytes())
        f.write(rank_full.T.tobytes())
    print(f"wrote {pc_path}, {os.path.getsize(pc_path)} bytes, from {len(e)} reference readings")
    # How far the reference readings sit from the class vectors on each axis:
    # the share of readings beyond the class vectors' own 5th and 95th
    # percentiles (0.1 if they were spread alike).
    lo5, hi95 = np.percentile(score[:, :n], [5, 95], axis=0)
    outside = ((proj < lo5) | (proj > hi95)).mean(0)
    print("readings outside the classes' 5-95% range, per PC: " + " ".join(f"{v:.2f}" for v in outside))

    for k in range(n):
        low = [name(idx[i]) for i in order[:12, k]]
        high = [name(idx[i]) for i in order[-12:, k][::-1]]
        print(f"\nPC{k + 1} ({pca.explained_variance_ratio_[k] * 100:.2f}%)")
        print("  low:  " + "; ".join(low))
        print("  high: " + "; ".join(high))
        # Group shares in each outer tenth.
        for end, rows in (("low tenth", order[: len(idx) // 10, k]), ("high tenth", order[-len(idx) // 10 :, k])):
            g = [idx[i] for i in rows]
            sound = np.mean([not binomial.match(labels[i]) for i in g])
            bird = np.mean([codes[i] != "no_ebird_code" for i in g])
            print(f"  {end}: with an eBird code (birds) {bird:.2f}, sound events {sound:.2f}, other {1 - bird - sound:.2f}")

    if spatial:
        # Check on a subsample: each reading's percentile (by the reference
        # quantiles) against its top class's percentile, for readings whose
        # top class has a softmax share of 0.3 or more.
        rng = np.random.default_rng(SEED)
        sp = np.load(spatial, mmap_mode="r")
        first = np.load(embs[0])["perch"].reshape(-1, D)
        pick = np.sort(rng.choice(len(sp), min(1500, len(sp)), replace=False))
        tops, keep_rows = [], []
        t = time.time()
        for s0 in range(0, len(pick), 25):
            rows = pick[s0 : s0 + 25]
            s = np.asarray(sp[rows], np.float32).reshape(len(rows), -1, D)
            s = s / (np.sqrt((s * s).sum(-1, keepdims=True)) + 1e-5)
            logit = (s @ P).reshape(len(rows), s.shape[1], C, 4).max(1)
            logit = (logit * W[None]).sum(-1) + bias
            m = logit.max(1, keepdims=True)
            prob = np.exp(logit - m) / np.exp(logit - m).sum(1, keepdims=True)
            tops.append(logit.argmax(1))
            keep_rows.append(prob.max(1) >= 0.3)
        top = np.concatenate(tops)
        keep = np.concatenate(keep_rows)
        print(f"\ncheck on {keep.sum()} of {len(pick)} sampled readings with a class at 0.3 or more ({time.time() - t:.0f} s):")
        pos = np.full(C, -1)
        pos[idx] = np.arange(len(idx))
        ok = keep & (pos[top] >= 0)
        pr = unit(first[pick[ok]].astype(np.float32)) @ axes[:n].T
        for k in range(n):
            p = np.interp(pr[:, k], ref[k], QUANTILES / 100)
            r = pct[pos[top[ok]], k]
            d = np.abs(p - r).mean()
            ds = np.abs(p - rng.permutation(r)).mean()
            c = np.corrcoef(p, r)[0, 1]
            print(f"  PC{k + 1}: mean gap {d:.3f}, shuffled {ds:.3f}, correlation {c:.2f}")


if __name__ == "__main__":
    main()
