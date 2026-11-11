"""A 2D map of every class Perch v2 knows, from its classifier weights.

Perch v2's classifier has 4 unit-length prototypes per class (P, from split.py), and a
non-negative weight per prototype (W). Class-to-class similarity is tried three ways:

  mean: cosine of the W-weighted mean of each class's prototypes
  best: cosine of each class's highest-weight prototype
  max:  the largest cosine between any prototype of one class and any of the other

Each is scored by how often a class's nearest other class is in the same genus (a
rough check that the space is organised by relatedness). BirdNET V2.4's dense
classifier weights (1024 x 6522) get the same score for comparison.

The 2D map is UMAP (cosine) on the best-scoring similarity. It writes:
  OUT_DIR/perch_v2_species_map.json   [{"i", "name", "x", "y"}], coordinates rounded
  OUT_DIR/perch_v2_species_map.png    a picture, birds / other species / sound events

It also checks how to place a detection on the map: for each window in WINDOWS_PREFIX
(saved by ref.py / ref_tf.py), whether the class with the nearest prototype to the
pooled `embedding` is the model's top-1 label.

  python species_map.py MODELS_DIR OUT_DIR [WINDOWS_PREFIX]
"""

import json
import os
import re
import sys

import numpy as np

models, outdir = sys.argv[1], sys.argv[2]
win_prefix = sys.argv[3] if len(sys.argv) > 3 else None

H = np.load(os.path.join(models, "perch_v2_head.npz"))
P = H["P"].reshape(1536, -1, 4).transpose(1, 2, 0)  # [class, proto, dim]
W = H["W"]
C = P.shape[0]
labels = [l.strip() for l in open(os.path.join(models, "labels.csv"), encoding="utf-8")][1:]
ebird = [l.strip() for l in open(os.path.join(models, "perch_v2_ebird_classes.csv"), encoding="utf-8")][1:]
binomial = re.compile(r"^[A-Z][a-z]+ [a-z-]+$")
is_species = np.array([bool(binomial.match(l)) for l in labels])
is_bird = np.array([e != "no_ebird_code" for e in ebird])
genus = np.array([l.split(" ")[0] if s else "" for l, s in zip(labels, is_species)])
dead = W.sum(1) == 0  # all 4 prototype weights are 0: the logit is the bias alone
print(f"{C} classes: {is_species.sum()} species names, {is_bird.sum()} with an eBird code, {(~is_species).sum()} other (sound events)")
print(f"{dead.sum()} classes have all-zero prototype weights and are left off the map: {[labels[i] for i in np.nonzero(dead)[0][:8]]} ...")
is_species &= ~dead


def unit(X):
    n = np.linalg.norm(X, axis=1, keepdims=True)
    return X / np.where(n == 0, 1, n)


def genus_nn(S, gen, mask):
    """Share of species (in mask, whose genus has 2+ species) whose nearest other class shares the genus."""
    S = S.copy()
    np.fill_diagonal(S, -np.inf)
    S[:, ~mask] = -np.inf
    nn = S.argmax(1)
    counts = {g: n for g, n in zip(*np.unique(gen[mask], return_counts=True))}
    rows = np.array([m and counts.get(g, 0) >= 2 for m, g in zip(mask, gen)])
    hit = gen[nn[rows]] == gen[rows]
    rand = np.mean([counts[g] - 1 for g in gen[rows]]) / (mask.sum() - 1)
    return hit.mean(), rows.sum(), rand


mean_v = unit((P * W[:, :, None]).sum(1))
best_v = P[np.arange(C), W.argmax(1)]
S_mean = mean_v @ mean_v.T
S_best = best_v @ best_v.T
S_max = np.full((C, C), -1.0, np.float32)
flat = P.reshape(-1, 1536)
for a in range(0, C, 500):
    blk = P[a : a + 500].reshape(-1, 1536) @ flat.T  # [500*4, C*4]
    S_max[a : a + 500] = blk.reshape(-1, 4, C, 4).max(axis=(1, 3))

results = {}
for name, S in (("mean", S_mean), ("best", S_best), ("max", S_max)):
    acc, n, rand = genus_nn(S, genus, is_species)
    results[name] = acc
    print(f"Perch {name:4s}: nearest class shares the genus for {acc:.3f} of {n} species (random: {rand:.4f})")

# BirdNET V2.4: dense layer weights, for comparison.
try:
    import onnx
    from onnx import numpy_helper

    bm = onnx.load(os.path.join(models, "BirdNET_v2.4_fp32_foldconv.onnx"))
    Wb = next(numpy_helper.to_array(t) for t in bm.graph.initializer if list(t.dims) == [1024, 6522]).T
    bl = [l.strip().split("_")[0] for l in open(os.path.join(models, "birdnet_labels.txt"), encoding="utf-8")]
    bsp = np.array([bool(binomial.match(l)) for l in bl])
    bgen = np.array([l.split(" ")[0] if s else "" for l, s in zip(bl, bsp)])
    Vb = unit(Wb)
    acc, n, rand = genus_nn(Vb @ Vb.T, bgen, bsp)
    print(f"BirdNET dense weights: nearest class shares the genus for {acc:.3f} of {n} species (random: {rand:.4f})")
    # Same check on Perch, restricted to classes BirdNET also has, for a like-for-like number.
    common = np.array([l in set(bl) for l in labels]) & is_species
    for name, S in (("mean", S_mean), ("best", S_best), ("max", S_max)):
        a2, n2, r2 = genus_nn(S, genus, common)
        print(f"  Perch {name} on the {common.sum()} species BirdNET also has: {a2:.3f} of {n2} (random {r2:.4f})")
    bcommon = np.array([l in set(labels) for l in bl]) & bsp
    a3, n3, r3 = genus_nn(Vb @ Vb.T, bgen, bcommon)
    print(f"  BirdNET on the same species: {a3:.3f} of {n3} (random {r3:.4f})")
except FileNotFoundError:
    Vb = None

# Placing a detection: nearest prototype to the pooled embedding vs the top-1 label.
if win_prefix:
    emb = np.fromfile(win_prefix + ".emb.f32", dtype="<f4").reshape(-1, 1536)
    lab = np.fromfile(win_prefix + ".label.f32", dtype="<f4").reshape(-1, C)
    sims = unit(emb) @ flat.T  # [N, C*4]
    near_class = sims.argmax(1) // 4
    top = lab.argmax(1)
    print(f"\nnearest prototype to the pooled embedding = top-1 label in {np.sum(near_class == top)} of {len(top)} windows")
    rank = [(np.argsort(-lab[i]) == near_class[i]).nonzero()[0][0] + 1 for i in range(len(top))]
    print("  rank of that class in the window's logits:", rank)

# The map: UMAP on the best similarity.
pick = max(results, key=results.get)
S = {"mean": S_mean, "best": S_best, "max": S_max}[pick]
D = np.clip(1 - S, 0, 2).astype(np.float64)
np.fill_diagonal(D, 0)
import umap  # noqa: E402

keep = ~dead
xy = np.zeros((C, 2))
xy[keep] = umap.UMAP(metric="precomputed", n_neighbors=15, min_dist=0.1, random_state=0).fit_transform(D[np.ix_(keep, keep)])
xy = (xy - xy.min(0)) / (xy.max(0) - xy.min(0))
mp = [{"i": i, "name": labels[i], "x": round(float(x), 4), "y": round(float(y), 4)} for i, (x, y) in enumerate(xy) if keep[i]]
path = os.path.join(outdir, "perch_v2_species_map.json")
json.dump(mp, open(path, "w", encoding="utf-8"), separators=(",", ":"), ensure_ascii=False)
print(f"\nmap from '{pick}' similarity: {path}, {os.path.getsize(path) / 1e3:.0f} kB")

# 2D check: nearest neighbour on the map shares the genus.
d2 = ((xy[:, None, :] - xy[None, :, :]) ** 2).sum(-1)
acc2, n2, _ = genus_nn(-d2, genus, is_species)
print(f"on the 2D map, the nearest class shares the genus for {acc2:.3f} of {n2} species")
if Vb is not None:
    bxy = umap.UMAP(metric="cosine", n_neighbors=15, min_dist=0.1, random_state=0).fit_transform(Vb)
    bd2 = ((bxy[:, None, :] - bxy[None, :, :]) ** 2).sum(-1)
    acc3, n3, _ = genus_nn(-bd2, bgen, bsp)
    print(f"BirdNET 2D map (UMAP, cosine): nearest class shares the genus for {acc3:.3f} of {n3} species")

import matplotlib  # noqa: E402

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

fig, ax = plt.subplots(figsize=(9, 9), dpi=110)
groups = [
    (is_bird & is_species, "#3b6ea8", "Birds"),
    (is_species & ~is_bird, "#d08a1e", "Other species"),
    (~is_species & keep, "#9a3a8a", "Sound events"),
]
for m, col, name in groups:
    ax.scatter(xy[m, 0], xy[m, 1], s=1.5, c=col, label=f"{name} ({m.sum()})", linewidths=0, alpha=0.7)
marks = {
    "Eptesicus serotinus": (6, 6),
    "Eptesicus nilssonii": (6, -10),
    "Megaptera novaeangliae": (6, 4),
    "Turdus merula": (-70, 6),
    "Rana temporaria": (6, -10),
    "Gryllus campestris": (6, 4),
}
for nm, off in marks.items():
    if nm in labels:
        i = labels.index(nm)
        ax.annotate(nm, xy[i], fontsize=7, xytext=off, textcoords="offset points")
        ax.scatter(*xy[i], s=12, c="black")
ax.set_xticks([])
ax.set_yticks([])
ax.set_title(f"Perch v2: {keep.sum()} classes, UMAP of classifier prototypes ('{pick}' similarity)")
ax.legend(markerscale=6, loc="lower left", fontsize=8)
fig.tight_layout()
fig.savefig(os.path.join(outdir, "perch_v2_species_map.png"))
print("wrote", os.path.join(outdir, "perch_v2_species_map.png"))
