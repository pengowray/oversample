"""Data for the live view with BirdNET+ V3.0: a UMAP layout of its classes
(birdnet-v3-umap.bin) and the principal components of its class rows
(birdnet-v3-pc-rows.bin). The V3.0 version of species_umap.py and
pc_bands.py, laid out like perch_live_data.py.

Class vector: the class's row of the pruned head's weight (the head is one
linear layer on the 1,280-value embedding), scaled to length 1, as
research/species-map/project_v3.py uses for birdnet-v3-map.bin. Every class
has a row, so none is left out.

  python -I v3_live_data.py MODEL.onnx LABELS.tsv TSNE_MAP.bin OUT_DIR EMB.npz [EMB.npz ...]

MODEL.onnx is BirdNET+_V3.0_Global_12K_FP16_pruned.onnx; LABELS.tsv is
research/birdnet-v3-bench/birdnet-v3-labels.tsv; TSNE_MAP.bin is
research/species-map/birdnet-v3-map.bin. EMB.npz files are
birdset_embed_v3.py's output for BirdSet's HSN and POW test sets (V3.0
embeddings of the recordings the V2.4 and Perch files used).

UMAP runs in node with the app's vendored umap-js (umap_js.cjs) on the class
rows after PCA to 50 dimensions, scaled back to length 1, as for Perch
(umap-learn is not installed on this machine).

Writes, in OUT_DIR:
  birdnet-v3-umap.bin     11,725 x 2 int16 (as birdnet-v3-map.bin), the layout
  birdnet-v3-pc-rows.bin  OPC1 file (pc_bands.py's layout), D = 1,280, C = 11,725
  v3-umap.png             the layout coloured by taxon group

Needs numpy, scikit-learn, matplotlib, onnx, and node.
"""

import os
import struct
import sys

import numpy as np
from sklearn.decomposition import PCA
from sklearn.neighbors import NearestNeighbors

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../species-map"))
from project_v3 import CLASSES, DIM, group_of, head, plot, read_labels, umap_js, unit  # noqa: E402

SEED = 1
K = 10
MAX_N = 16
# 12 components, as BirdNET V2.4's and Perch's files have.
N = 12
QUANTILES = np.linspace(0, 100, 21)
SETTINGS = [(15, 0.1), (50, 0.1)]


def read_map(path):
    raw = np.frombuffer(open(path, "rb").read(), "<i2").reshape(-1, 2)
    return raw.astype(np.float32) / 32767


def compactness(xy, p, bands=2):
    nn = NearestNeighbors(n_neighbors=K + 1).fit(xy)
    _, idx = nn.kneighbors(xy)
    idx = idx[:, 1:]
    out = []
    for k in range(p.shape[1]):
        band = np.minimum(np.floor(p[:, k] * bands), bands - 1)
        out.append(float((band[idx] == band[:, None]).mean()))
    return out


def nth_neighbour(xy, n):
    d, _ = NearestNeighbors(n_neighbors=n + 1).fit(xy).kneighbors(xy)
    return float(np.median(d[:, n]))


def main():
    model, labels_tsv, tsne_path, out_dir, *embs = sys.argv[1:]
    labels = read_labels(labels_tsv)
    sci = [r[1] for r in labels]
    eng = [r[2] for r in labels]
    groups = [group_of(r[3]) for r in labels]
    W, bias, s = head(model)
    V = unit(W)

    def name(i):
        return sci[i] if eng[i] == sci[i] else f"{eng[i]} ({sci[i]})"

    pca = PCA(n_components=MAX_N, random_state=SEED).fit(V)
    axes = pca.components_.astype(np.float32)
    score = V @ axes.T
    order = np.argsort(score, axis=0)
    rank = np.empty_like(order)
    for k in range(MAX_N):
        rank[order[:, k], k] = np.arange(CLASSES)
    pct = rank / (CLASSES - 1)
    rng = np.random.default_rng(SEED)
    shuffled = PCA(n_components=4, random_state=SEED).fit(np.column_stack([rng.permutation(V[:, j]) for j in range(DIM)]))
    print("variance, PC1..PC%d (%%): %s" % (MAX_N, " ".join(f"{v * 100:.2f}" for v in pca.explained_variance_ratio_)))
    print("same PCA with each column shuffled (%%): %s" % " ".join(f"{v * 100:.2f}" for v in shuffled.explained_variance_ratio_))

    tsne = read_map(tsne_path)
    assert len(tsne) == CLASSES
    layouts = {"t-SNE": tsne}
    V50 = unit(PCA(n_components=50, random_state=SEED).fit_transform(V))
    for nn, md in SETTINGS:
        xy = umap_js(V50, nn, md)
        xy = xy - np.median(xy, axis=0)
        xy = np.clip(xy / np.percentile(np.abs(xy), 99.5), -1, 1)
        layouts[f"UMAP {nn}/{md}"] = xy.astype(np.float32)

    print(f"\nclass rows, halves: share of {K} nearest neighbours in the same half, PC1..PC{MAX_N}")
    comp = {}
    for lname, xy in layouts.items():
        c = compactness(xy, pct)
        comp[lname] = c
        print(f"  {lname:14s} mean {np.mean(c[:N]):.2f} (PC1-{N}):  " + " ".join(f"{v:.2f}" for v in c))
    best = max((l for l in layouts if l != "t-SNE"), key=lambda l: np.mean(comp[l][:N]))
    print(f"chosen UMAP: {best}")
    xy = layouts[best]
    lo, hi = xy.min(0), xy.max(0)
    xy = (xy - (lo + hi) / 2) / ((hi - lo).max() / 2)
    umap_path = os.path.join(out_dir, "birdnet-v3-umap.bin")
    np.round(xy * 32767).astype("<i2").tofile(umap_path)
    print(f"wrote {umap_path}")
    d_tsne, d_umap = nth_neighbour(tsne, K), nth_neighbour(xy, K)
    print(f"median distance to the {K}th nearest class: t-SNE {d_tsne:.4f}, UMAP {d_umap:.4f}, ratio {d_umap / d_tsne:.2f}")
    plot(xy, groups, os.path.join(out_dir, "v3-umap.png"), f"BirdNET+ V3.0 classes, UMAP ({best}) of the head's rows")

    # Reference recordings' embeddings, projected (silent windows left out).
    loaded = [np.load(p) for p in embs]
    keep = np.concatenate([~d["silent"] for d in loaded])
    e = np.concatenate([d["v3"] for d in loaded])[keep].astype(np.float32)
    top = np.concatenate([d["top"] for d in loaded])[keep]
    topconf = np.concatenate([d["topconf"] for d in loaded])[keep]
    proj = unit(e) @ axes[:N].T
    ref = np.percentile(proj, QUANTILES, axis=0).T.astype(np.float32)
    rank_u8 = np.round(pct[:, :N] * 255).astype(np.uint8)
    pc_path = os.path.join(out_dir, "birdnet-v3-pc-rows.bin")
    with open(pc_path, "wb") as f:
        f.write(b"OPC1")
        f.write(struct.pack("<4H", N, DIM, CLASSES, len(QUANTILES)))
        f.write(pca.explained_variance_ratio_[:N].astype("<f4").tobytes())
        f.write(axes[:N].astype("<f4").tobytes())
        f.write(ref.astype("<f4").tobytes())
        f.write(rank_u8.T.tobytes())
    print(f"wrote {pc_path}, {os.path.getsize(pc_path)} bytes, from {len(e)} reference readings")
    lo5, hi95 = np.percentile(score[:, :N], [5, 95], axis=0)
    outside = ((proj < lo5) | (proj > hi95)).mean(0)
    print("readings outside the classes' 5-95% range, per PC: " + " ".join(f"{v:.2f}" for v in outside))
    best_conf = topconf[:, 0]
    print(
        "reference windows' best confidence: median %.3f; share at 0.1 or more %.2f, 0.25 or more %.2f, 0.5 or more %.2f"
        % (np.median(best_conf), (best_conf >= 0.1).mean(), (best_conf >= 0.25).mean(), (best_conf >= 0.5).mean())
    )
    print(
        "second best at 0.1 or more in %.2f of windows; median second best %.3f"
        % ((topconf[:, 1] >= 0.1).mean(), np.median(topconf[:, 1]))
    )

    tenth = CLASSES // 10
    for k in range(N):
        low = [name(i) for i in order[:12, k]]
        high = [name(i) for i in order[-12:, k][::-1]]
        print(f"\nPC{k + 1} ({pca.explained_variance_ratio_[k] * 100:.2f}%), compactness on {best} {comp[best][k]:.2f}")
        print("  low:  " + "; ".join(low))
        print("  high: " + "; ".join(high))
        for end, rows in (("low tenth", order[:tenth, k]), ("high tenth", order[-tenth:, k])):
            g = [groups[i] for i in rows]
            parts = ", ".join(f"{x} {g.count(x) / len(g):.2f}" for x in ("birds", "insects", "amphibians", "mammals", "other animals", "sounds") if g.count(x))
            print(f"  {end}: {parts}")

    # Check: for confident readings, the reading's percentile (by the
    # reference quantiles) against its top class's percentile, and against a
    # shuffle.
    sel = best_conf >= 0.5
    print(f"\ncheck on {sel.sum()} readings with a class at 0.5 or more:")
    for k in range(N):
        p = np.interp(proj[sel, k], ref[k], QUANTILES / 100)
        r = pct[top[sel, 0], k]
        d = np.abs(p - r).mean()
        ds = np.abs(p - rng.permutation(r)).mean()
        c = np.corrcoef(p, r)[0, 1]
        print(f"  PC{k + 1}: mean gap {d:.3f}, shuffled {ds:.3f}, correlation {c:.2f}")


if __name__ == "__main__":
    main()
