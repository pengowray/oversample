"""Make a 2-D map of Perch 2.0's 14,795 classes from the model's own weights,
after comparing several projections.

Perch 2.0's released classifier has no single weight row per class. It is a
prototype head: 4 unit-length prototypes per class (P), a non-negative weight
per prototype (W) and a bias. The class vector used here is the W-weighted mean
of a class's 4 prototypes, L2-normalised; research/perch.md section 5 found it
groups relatives far better than the best single prototype. Distances are
cosine.

  python -I project_perch.py HEAD.npz LABELS.csv EBIRD_CLASSES.csv EBIRD_TAXONOMY.csv OUT_DIR

HEAD.npz is perch_v2_head.npz from research/perch-bench/split.py (P: 1536 x
59,180, W: 14,795 x 4, b). LABELS.csv and EBIRD_CLASSES.csv are the two files in
cgeorgiaw/Perch `assets/`. EBIRD_TAXONOMY.csv is
https://api.ebird.org/v2/ref/taxonomy/ebird?fmt=csv (gives each bird's order).

Writes, in OUT_DIR:
  perch-map.bin     14,795 x 2 int16, little endian: x then y per class, in
                    label order, scaled to -32767..32767. Classes whose 4
                    prototype weights are all 0 (they never score above their
                    bias) are written as (-32768, -32768), which the app
                    reads as "no position" and does not draw.
  perch-check.txt   the comparison table.
  perch-map.png     a scatter of the chosen map.

Needs numpy, scikit-learn, umap-learn, matplotlib. Takes about 20 minutes.
"""

import csv
import os
import re
import sys
import time

import numpy as np
from sklearn.decomposition import PCA
from sklearn.manifold import TSNE
from sklearn.neighbors import NearestNeighbors

SEED = 1
K = 5


def unit(x):
    n = np.linalg.norm(x, axis=1, keepdims=True)
    return x / np.where(n == 0, 1, n)


def share(points, groups, rows, k, metric):
    """For each row in `rows`, the share of its k nearest neighbours (among all
    points) whose group equals its own. Mean over rows."""
    nn = NearestNeighbors(n_neighbors=k + 1, metric=metric).fit(points)
    _, idx = nn.kneighbors(points[rows])
    g = groups
    return float(np.mean([np.mean(g[row[1:]] == g[i]) for i, row in zip(rows, idx)]))


def random_share(groups, rows, rng):
    a = rng.choice(rows, 50000)
    b = rng.integers(0, len(groups), 50000)
    keep = a != b
    return float(np.mean(groups[a[keep]] == groups[b[keep]]))


def main():
    head, labels_csv, ebird_csv, tax_csv, out = sys.argv[1:6]
    os.makedirs(out, exist_ok=True)
    h = np.load(head)
    P, W = h["P"], h["W"]
    C = W.shape[0]
    P = P.reshape(P.shape[0], C, 4).transpose(1, 2, 0)  # class, prototype, dim
    labels = [l.strip() for l in open(labels_csv, encoding="utf-8")][1:]
    codes = [l.strip() for l in open(ebird_csv, encoding="utf-8")][1:]
    assert len(labels) == C == len(codes), (len(labels), C, len(codes))
    order_of = {r["SPECIES_CODE"]: r["ORDER"] for r in csv.DictReader(open(tax_csv, encoding="utf-8"))}

    binomial = re.compile(r"^[A-Z][a-z]+ [a-z-]+")
    live = W.sum(1) > 0
    species = np.array([bool(binomial.match(l)) for l in labels])
    bird = np.array([c != "no_ebird_code" for c in codes])
    genus = np.array([l.split(" ")[0] if s else f"sound:{l}" for l, s in zip(labels, species)])
    group = np.where(~species, "sound", np.where(bird, "bird", "other"))
    order = np.array([order_of.get(c, f"?{i}") if b else f"?{i}" for i, (c, b) in enumerate(zip(codes, bird))])

    idx = np.nonzero(live)[0]
    V = unit((P * W[:, :, None]).sum(1))[idx]
    g_genus, g_group, g_order = genus[idx], group[idx], order[idx]
    sp_rows = np.nonzero(species[idx])[0]
    bird_rows = np.nonzero((bird & species)[idx])[0]
    all_rows = np.arange(len(idx))
    known_order = np.mean([not o.startswith("?") for o in g_order[bird_rows]])
    print(f"{C} classes, {live.sum()} live; {len(sp_rows)} species, {len(bird_rows)} birds "
          f"({known_order:.3f} with an order); {(g_group == 'sound').sum()} sound events", flush=True)

    rng = np.random.default_rng(SEED)
    lines = [
        f"Perch 2.0, {len(idx)} live classes of {C}. Class vector: weighted mean of the 4 prototypes, cosine.",
        f"Share of the {K} nearest neighbours that match: genus (over {len(sp_rows)} species), "
        f"bird order (over {len(bird_rows)} birds), group (bird / other species / sound event, all classes).",
        "",
        f"{'method':34s} {'genus':>6s} {'order':>6s} {'group':>6s} {'secs':>6s}",
    ]

    def row(name, pts, metric, secs=0.0):
        r = (share(pts, g_genus, sp_rows, K, metric), share(pts, g_order, bird_rows, K, metric),
             share(pts, g_group, all_rows, K, metric))
        lines.append(f"{name:34s} {r[0]:6.3f} {r[1]:6.3f} {r[2]:6.3f} {secs:6.0f}")
        print(lines[-1], flush=True)
        return r

    lines.append(f"{'random pairs':34s} {random_share(g_genus, sp_rows, rng):6.4f} "
                 f"{random_share(g_order, bird_rows, rng):6.3f} {random_share(g_group, all_rows, rng):6.3f}")
    row("full 1,536-d vectors", V, "cosine")
    X50 = PCA(n_components=50, random_state=SEED).fit_transform(V)
    row("PCA to 50-d", X50, "euclidean")
    t = time.time()
    X2 = PCA(n_components=2, random_state=SEED).fit_transform(V)
    row("PCA to 2-d", X2, "euclidean", time.time() - t)

    maps = {}
    for perp in (30, 50, 100):
        for src, X, metric in (("PCA-50", X50, "euclidean"), ("full", V, "cosine")):
            if src == "full" and perp != 30:
                continue
            t = time.time()
            xy = TSNE(n_components=2, metric=metric, perplexity=perp, init="pca",
                      random_state=SEED).fit_transform(X)
            name = f"t-SNE p{perp} ({src})"
            maps[name] = (xy, row(name, xy, "euclidean", time.time() - t))
    import umap

    for nn in (15, 50):
        for src, X in (("PCA-50", X50), ("full", V)):
            t = time.time()
            xy = umap.UMAP(n_neighbors=nn, min_dist=0.1, metric="cosine",
                           random_state=SEED).fit_transform(X)
            name = f"UMAP n{nn} cosine ({src})"
            maps[name] = (xy, row(name, xy, "euclidean", time.time() - t))

    # Pick: best genus share; order and group only break near-ties (within 0.01).
    best_genus = max(r[0] for _, r in maps.values())
    pick = max((n for n, (_, r) in maps.items() if r[0] >= best_genus - 0.01),
               key=lambda n: maps[n][1][1] + maps[n][1][2])
    lines += ["", f"Chosen: {pick}"]
    xy = maps[pick][0]
    xy = xy - xy.mean(axis=0)
    full = np.zeros((C, 2))
    full[idx] = xy / np.abs(xy).max()
    scaled = np.round(full * 32767).astype("<i2")
    scaled[~live] = -32768
    scaled.tofile(os.path.join(out, "perch-map.bin"))

    # Placement radius: distance from each class to its 3rd nearest map
    # neighbour, in map units (the map is 2 wide), for comparing with BirdNET's.
    m = full[idx]
    d, _ = NearestNeighbors(n_neighbors=4).fit(m).kneighbors(m)
    lines.append(f"Distance to 3rd nearest class, map units: median {np.median(d[:, 3]):.4f}, "
                 f"90th percentile {np.percentile(d[:, 3], 90):.4f}")
    open(os.path.join(out, "perch-check.txt"), "w").write("\n".join(lines) + "\n")
    print("\n".join(lines[-2:]))

    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    fig, ax = plt.subplots(figsize=(10, 10), dpi=110)
    for gname, col in (("bird", "#3b6ea8"), ("other", "#d08a1e"), ("sound", "#9a3a8a")):
        s = g_group == gname
        ax.scatter(m[s, 0], m[s, 1], s=1.5, c=col, label=f"{gname} ({s.sum()})", linewidths=0)
    for nm in ("Turdus merula", "Turdus philomelos", "Rana temporaria", "Gryllus campestris",
               "Megaptera novaeangliae", "Eptesicus serotinus", "Parus major", "Strix aluco"):
        if nm in labels:
            i = labels.index(nm)
            ax.annotate(nm, full[i], fontsize=7, xytext=(4, 4), textcoords="offset points")
            ax.scatter(*full[i], s=10, c="black")
    ax.set_title(f"Perch 2.0 species map: {pick}")
    ax.legend(markerscale=6, fontsize=8)
    fig.tight_layout()
    fig.savefig(os.path.join(out, "perch-map.png"))


if __name__ == "__main__":
    main()
